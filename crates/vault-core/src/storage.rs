use crate::{api::{ArchiveItem, CaptureSettings}, secure_fs::{self, DirectoryGuard, Identity}};
use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs::File, path::PathBuf, time::Duration};
use uuid::Uuid;

pub const MAX_PENDING: u64 = 1024;
pub const RETRY_WINDOW_MS: u64 = 40 * 60 * 1000;
const APP_ID: i64 = 0x43564C54;

pub fn now_ms() -> u64 { crate::capture::probe::windows::unix_ms() }
pub fn event_date(ms: u64) -> Result<String, String> {
    let ms = i64::try_from(ms).map_err(|_| "事件时间溢出")?;
    Local.timestamp_millis_opt(ms).single().map(|t| t.format("%Y-%m-%d").to_string()).ok_or("事件时间无效".into())
}
fn db_error(e: rusqlite::Error) -> String { format!("持久数据库操作失败：{e}") }

#[derive(Clone)]
pub struct Task {
    pub id: String,
    pub source: String,
    pub identity: Identity,
    pub display_name: String,
    pub extension: String,
    pub detected: u64,
    pub date: String,
    pub deadline: u64,
    pub attempts: u64,
    pub state: String,
    pub stage_rel: String,
    pub final_rel: String,
    pub entity_id: String,
    pub digest: Option<String>,
    pub size: u64,
}
#[derive(Clone)]
pub struct Entity { pub id: String, pub rel: String, pub digest: String, pub size: u64, pub state: String }

pub struct Store {
    pub root: PathBuf,
    pub configured_root: PathBuf,
    pub settings: CaptureSettings,
    pub connection: Connection,
    pub _root_guard: DirectoryGuard,
    pub _root_lock: File,
    pub _db_guard: secure_fs::Opened,
    pub _journal_guard: secure_fs::Opened,
}
impl Store {
    pub fn open(path: String) -> Result<Self, String> {
        let configured_root = secure_fs::local_path(&path)?;
        let guard = DirectoryGuard::acquire(&configured_root, true)?;
        guard.validate()?;
        let root=guard.path.clone();
        let lock = secure_fs::lock_root(&root)?;
        DirectoryGuard::acquire(&root.join("staging"), true)?;
        DirectoryGuard::acquire(&root.join("archive"), true)?;
        let db = root.join("vault.sqlite3");
        for name in ["vault.sqlite3", "vault.sqlite3-journal", "vault.sqlite3-wal", "vault.sqlite3-shm"] {
            secure_fs::reject_special_file(&root.join(name))?;
        }
        if !db.try_exists().map_err(|e|e.to_string())? { drop(secure_fs::create_owned(&db)?); }
        let db_guard = secure_fs::Opened::source(&db,true)?;
        secure_fs::ensure_single_link(&db_guard.file)?;
        let connection = Connection::open(&db).map_err(db_error)?;
        connection.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        let app: i64 = connection.query_row("PRAGMA application_id", [], |r| r.get(0)).map_err(db_error)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(db_error)?;
        let tables: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).map_err(db_error)?;
        if (app != APP_ID && (app != 0 || tables > 0)) || version > 1 {
            return Err("所选目录已有其他或更新版本数据库；不会自动迁移或覆盖".into());
        }
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL;").map_err(db_error)?;
        // 持久 rollback journal 避免 SQLite 在每个事务后删除/重建 sidecar。
        // 创建后锁住 inode，拒绝把 journal 换成 reparse/hardlink；SQLite 可正常写/截断。
        let journal = root.join("vault.sqlite3-journal");
        if !journal.try_exists().map_err(|e|e.to_string())? { drop(secure_fs::create_owned(&journal)?); }
        let journal_guard = secure_fs::Opened::source(&journal,true)?;
        secure_fs::ensure_single_link(&journal_guard.file)?;
        if app == 0 {
            connection.execute_batch(&format!("BEGIN IMMEDIATE;
                CREATE TABLE config(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE entities(id TEXT PRIMARY KEY, digest TEXT NOT NULL, size INTEGER NOT NULL,
                    rel TEXT NOT NULL UNIQUE, state TEXT NOT NULL, error TEXT);
                CREATE UNIQUE INDEX entity_content ON entities(digest,size);
                CREATE TABLE tasks(id TEXT PRIMARY KEY, source TEXT NOT NULL, identity TEXT NOT NULL,
                    display_name TEXT NOT NULL, extension TEXT NOT NULL, detected INTEGER NOT NULL,
                    date TEXT NOT NULL, state TEXT NOT NULL, deadline INTEGER NOT NULL, next_attempt INTEGER NOT NULL,
                    attempts INTEGER NOT NULL DEFAULT 0, stage_rel TEXT NOT NULL UNIQUE, final_rel TEXT NOT NULL UNIQUE,
                    entity_id TEXT NOT NULL, digest TEXT, size INTEGER NOT NULL, error TEXT, UNIQUE(date,identity));
                CREATE INDEX tasks_due ON tasks(state,next_attempt);
                CREATE TABLE usages(date TEXT NOT NULL, entity_id TEXT NOT NULL REFERENCES entities(id),
                    detected INTEGER NOT NULL, task_id TEXT NOT NULL REFERENCES tasks(id), display_name TEXT NOT NULL,
                    PRIMARY KEY(date,entity_id));
                CREATE TABLE aliases(date TEXT NOT NULL, identity TEXT NOT NULL, source TEXT NOT NULL,
                    display_name TEXT NOT NULL, PRIMARY KEY(date,identity,source));
                PRAGMA application_id={APP_ID}; PRAGMA user_version=1; COMMIT;" )).map_err(db_error)?;
        } else if version != 1 {
            return Err("数据库结构版本不受支持，不会静默迁移".into());
        }
        let json: Option<String> = connection.query_row("SELECT value FROM config WHERE key='capture'", [], |r| r.get(0)).optional().map_err(db_error)?;
        let settings = match json {
            Some(json) => serde_json::from_str(&json).map_err(|e| format!("监测设置损坏：{e}"))?,
            None => CaptureSettings::default(),
        };
        let settings = crate::capture::validate_settings(settings)?;
        connection.execute("INSERT OR IGNORE INTO config(key,value) VALUES('capture',?1)", [serde_json::to_string(&settings).map_err(|e| e.to_string())?]).map_err(db_error)?;
        // task-known staging/final retained for recovery. No directory sweeps and no source scans.
        connection.execute("UPDATE tasks SET state=CASE WHEN digest IS NOT NULL THEN 'staged' ELSE 'pending' END,
            next_attempt=?1, error='上次进程在归档中断；正在恢复' WHERE state IN ('copying','pending','staged')", [now_ms()]).map_err(db_error)?;
        Ok(Self { root, configured_root,settings, connection, _root_guard: guard, _root_lock: lock, _db_guard:db_guard, _journal_guard:journal_guard })
    }
    pub fn validate(&self)->Result<(),String> {self._root_guard.validate()}
    pub fn display_root(&self)->Result<PathBuf,String> {self._root_guard.display_path()}
    pub fn set_settings(&mut self, settings: CaptureSettings) -> Result<(), String> {
        self.validate()?;
        let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        self.connection.execute("INSERT INTO config(key,value) VALUES('capture',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(db_error)?;
        self.settings = settings;
        Ok(())
    }
    pub fn counts(&self) -> Result<(u64, u64), String> {
        self.validate()?;
        self.connection.query_row("SELECT coalesce(sum(state IN ('pending','copying','staged')),0), coalesce(sum(state='failed'),0) FROM tasks", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_error)
    }
    pub fn accept(&mut self, source: String, identity: Identity, size: u64, detected: u64) -> Result<bool, String> {
        self.validate()?;
        let date = event_date(detected)?;
        let identity = serde_json::to_string(&identity).map_err(|e| e.to_string())?;
        let name = source.rsplit('\\').next().unwrap_or(&source).to_owned();
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        // 同日同源身份合并；仍保留原名/来源别名。不会因事件风暴重新开启失败任务。
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE date=?1 AND identity=?2)", params![date, identity], |r| r.get(0)).map_err(db_error)?;
        if !exists && self.counts()?.0 >= MAX_PENDING { return Err("捕获任务队列已满（1024），未接受新任务".into()); }
        let tx = self.connection.transaction().map_err(db_error)?;
        tx.execute("INSERT OR IGNORE INTO aliases(date,identity,source,display_name) VALUES(?1,?2,?3,?4)", params![date, identity, source, name]).map_err(db_error)?;
        if exists { self._root_guard.validate()?; tx.commit().map_err(db_error)?; return Ok(false); }
        let id = Uuid::new_v4().to_string();
        let entity_id = Uuid::new_v4().to_string();
        let original_stem=name.rsplit_once('.').map(|(stem,_)|stem).unwrap_or(&name);
        let stem = secure_fs::safe_filename(original_stem);
        // 截短只影响安全文件名主体，保持真实格式扩展名与唯一实体后缀。
        let suffix = if extension.is_empty() { String::new() } else { format!(".{extension}") };
        let final_rel = format!("archive/{date}/未分类/{stem}--{entity_id}{suffix}");
        let stage_rel = format!("staging/{id}.part");
        let now = now_ms();
        tx.execute("INSERT INTO tasks(id,source,identity,display_name,extension,detected,date,state,deadline,next_attempt,stage_rel,final_rel,entity_id,size)
            VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9,?10,?11,?12,?13)",
            params![id, source, identity, name, extension, detected, date, now.saturating_add(RETRY_WINDOW_MS), now, stage_rel, final_rel, entity_id, size]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)?;
        Ok(true)
    }
    pub fn due(&mut self) -> Result<Option<Task>, String> {
        self.validate()?;
        let id: Option<String> = self.connection.query_row("SELECT id FROM tasks WHERE state IN ('pending','staged') AND next_attempt<=?1 ORDER BY next_attempt,detected LIMIT 1", [now_ms()], |r| r.get(0)).optional().map_err(db_error)?;
        let Some(id) = id else { return Ok(None); };
        let task = self.task(&id)?;
        self.connection.execute("UPDATE tasks SET state='copying',attempts=attempts+1 WHERE id=?1", [&id]).map_err(db_error)?;
        Ok(Some(Task { attempts: task.attempts + 1, ..task }))
    }
    pub fn task(&self, id: &str) -> Result<Task, String> {
        self.validate()?;
        let result = self.connection.query_row("SELECT id,source,identity,display_name,extension,detected,date,deadline,attempts,state,stage_rel,final_rel,entity_id,digest,size FROM tasks WHERE id=?1", [id], |r| {
            Ok((Task { id: r.get(0)?, source: r.get(1)?, identity: Identity { volume_serial: 0, volume_name: String::new(), file_id: [0;16], created: 0 },
                display_name: r.get(3)?, extension: r.get(4)?, detected: r.get(5)?, date: r.get(6)?, deadline: r.get(7)?, attempts: r.get(8)?, state: r.get(9)?, stage_rel: r.get(10)?, final_rel: r.get(11)?, entity_id: r.get(12)?, digest: r.get(13)?, size: r.get(14)? }, r.get::<_, String>(2)?))
        }).map_err(db_error)?;
        let (mut task, identity) = result;
        task.identity = serde_json::from_str(&identity).map_err(|e| format!("源身份记录损坏：{e}"))?;
        Ok(task)
    }
    pub fn staged(&self, task: &Task, digest: &str, size: u64) -> Result<(), String> {
        self.validate()?;
        self.connection.execute("UPDATE tasks SET state='staged',digest=?2,size=?3,error=NULL WHERE id=?1", params![task.id, digest, size]).map_err(db_error)?;
        Ok(())
    }
    pub fn by_content(&self, digest: &str, size: u64) -> Result<Option<Entity>, String> {
        self.validate()?;
        self.connection.query_row("SELECT id,rel,digest,size,state FROM entities WHERE digest=?1 AND size=?2", params![digest, size], entity_row).optional().map_err(db_error)
    }
    pub fn entity(&self, id: &str) -> Result<Entity, String> {
        self.validate()?;
        self.connection.query_row("SELECT id,rel,digest,size,state FROM entities WHERE id=?1", [id], entity_row).map_err(db_error)
    }
    pub fn finish(&mut self, task: &Task, entity: &Entity, insert: bool) -> Result<(), String> {
        self.validate()?;
        let tx = self.connection.transaction().map_err(db_error)?;
        if !insert {
            let available:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM entities WHERE id=?1 AND state='available' AND digest=?2 AND size=?3)",params![entity.id,entity.digest,entity.size],|r|r.get(0)).map_err(db_error)?;
            if !available {return Err("去重实体在提交前已失效，拒绝建立成功使用记录".into());}
        }
        if insert {
            tx.execute("INSERT INTO entities(id,rel,digest,size,state) VALUES(?1,?2,?3,?4,'available')", params![entity.id, entity.rel, entity.digest, entity.size]).map_err(db_error)?;
        }
        tx.execute("INSERT INTO usages(date,entity_id,detected,task_id,display_name) VALUES(?1,?2,?3,?4,?5)
            ON CONFLICT(date,entity_id) DO UPDATE SET
            task_id=CASE WHEN excluded.detected<usages.detected THEN excluded.task_id ELSE usages.task_id END,
            display_name=CASE WHEN excluded.detected<usages.detected THEN excluded.display_name ELSE usages.display_name END,
            detected=min(usages.detected,excluded.detected)", params![task.date, entity.id, task.detected, task.id, task.display_name]).map_err(db_error)?;
        tx.execute("UPDATE tasks SET state='done',entity_id=?2,digest=?3,size=?4,error=NULL WHERE id=?1", params![task.id, entity.id, entity.digest, entity.size]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    pub fn invalidate(&mut self, id: &str, error: &str) -> Result<(), String> {
        self.validate()?;
        // 包括复用此实体的使用记录；异常实体与关联任务事务性转为失败。
        let tx=self.connection.transaction().map_err(db_error)?;
        tx.execute("UPDATE entities SET state='failed',error=?2 WHERE id=?1", params![id,error]).map_err(db_error)?;
        tx.execute("UPDATE tasks SET state='failed',error=?2 WHERE entity_id=?1 AND state='done'", params![id,error]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    pub fn fail(&self, task: &Task, error: &str, terminal: bool) -> Result<(), String> {
        self.validate()?;
        let now = now_ms();
        let failed = terminal || now >= task.deadline;
        let seconds = match task.attempts { 0..=2 => 2, 3..=5 => 5, 6..=8 => 15, 9..=12 => 30, 13..=18 => 60, _ => 120 };
        let next = now.saturating_add(seconds * 1000).min(task.deadline);
        self.connection.execute("UPDATE tasks SET state=CASE WHEN ?2 THEN 'failed' WHEN digest IS NOT NULL THEN 'staged' ELSE 'pending' END,next_attempt=?3,error=?4 WHERE id=?1 AND state!='done'", params![task.id,
            failed, next, error]).map_err(db_error)?;
        Ok(())
    }
    pub fn retry(&self, id: &str) -> Result<(), String> {
        self.validate()?;
        let task = self.task(id)?;
        if task.state != "failed" { return Err("只能手动重试失败的捕获任务".into()); }
        if self.counts()?.0 >= MAX_PENDING { return Err("捕获任务队列已满".into()); }
        // 不改 detected/date/identity；重新挂载的同盘符不会取代原始卷/文件身份。
        let now = now_ms();
        self.connection.execute("UPDATE tasks SET state=CASE WHEN digest IS NULL THEN 'pending' ELSE 'staged' END,deadline=?2,next_attempt=?3,attempts=0,error=NULL WHERE id=?1", params![id,now.saturating_add(RETRY_WINDOW_MS),now]).map_err(db_error)?;
        Ok(())
    }
    pub fn today(&self) -> Result<Vec<ArchiveItem>, String> {
        self.validate()?;
        let date = event_date(now_ms())?;
        let mut stmt = self.connection.prepare("SELECT e.id,u.display_name,t.extension,e.size,u.detected,'available',NULL,NULL
            FROM usages u JOIN entities e ON e.id=u.entity_id JOIN tasks t ON t.id=u.task_id
            WHERE u.date=?1 AND e.state='available'
            UNION ALL SELECT id,display_name,extension,size,detected,CASE WHEN state='failed' THEN 'failed' ELSE 'pending' END,id,error
            FROM tasks WHERE date=?1 AND state IN ('pending','copying','staged','failed') ORDER BY 5,1").map_err(db_error)?;
        let rows=stmt.query_map([date], |r| Ok(ArchiveItem { id:r.get(0)?,display_name:r.get(1)?,extension:r.get(2)?,size:r.get(3)?,detected_unix_ms:r.get(4)?,state:r.get(5)?,task_id:r.get(6)?,error:r.get(7)? })).map_err(db_error)?;
        let collected=rows.collect::<Result<Vec<_>,_>>().map_err(db_error);
        collected
    }
}
fn entity_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    Ok(Entity { id:r.get(0)?,rel:r.get(1)?,digest:r.get(2)?,size:r.get(3)?,state:r.get(4)? })
}
