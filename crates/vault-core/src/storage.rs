use crate::{api::*, config, library::NameMatcher, schema::{self, db_error}, secure_fs::{self, DirectoryGuard, Identity}, timetable};
use rusqlite::{params, params_from_iter, types::{Type, Value}, Connection, OptionalExtension, TransactionBehavior};
use std::{fs::File, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};
use uuid::Uuid;

pub const MAX_PENDING: u64 = 1024;
pub const RETRY_WINDOW_MS: u64 = 40 * 60 * 1000;
const ENTITY_COLUMNS: &str = "id,rel,digest,size,state,display_name,extension,archived,error,trash_rel,deleted_at,owned_identity";

pub fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
pub fn event_date(ms: u64) -> Result<String, String> { timetable::event_date(ms) }

#[derive(Clone)]
pub struct Task {
    pub id: String, pub source: String, pub identity: Identity, pub display_name: String,
    pub extension: String, pub detected: u64, pub date: String, pub course_id: String,
    pub subject_name: String, pub deadline: u64, pub attempts: u64, pub state: String,
    pub stage_rel: String, pub final_rel: String, pub entity_id: String,
    pub digest: Option<String>, pub size: u64,
}
#[derive(Clone)]
pub struct Entity {
    pub id: String, pub rel: String, pub digest: String, pub size: u64, pub state: String,
    pub display_name: String, pub extension: String, pub archived: u64, pub error: Option<String>,
    pub trash_rel: Option<String>, pub deleted_at: Option<u64>, pub owned_identity: Option<Identity>,
}
#[derive(Clone)]
pub(crate) struct FileIntent {
    pub entity_id: String, pub kind: String, pub from_rel: String, pub to_rel: Option<String>,
    pub identity: Identity, pub digest: String, pub size: u64,
}
#[derive(Clone)]
pub(crate) struct ThumbnailRecord {
    pub entity_id: String, pub rel: String, pub source_digest: String, pub png_digest: String,
    pub identity: Identity, pub size: u64, pub created: u64, pub used: u64,
}

pub struct Store {
    pub root: PathBuf,
    pub configured_root: PathBuf,
    pub settings: CaptureSettings,
    pub bundle: ConfigBundle,
    pub connection: Connection,
    pub startup_errors: Vec<String>,
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
        let root = guard.path.clone();
        let lock = secure_fs::lock_root(&root)?;
        for name in ["staging", "archive", "recovery", "cache/thumbnails"] { DirectoryGuard::acquire(&root.join(name), true)?; }
        let db = root.join("vault.sqlite3");
        for name in ["vault.sqlite3", "vault.sqlite3-journal", "vault.sqlite3-wal", "vault.sqlite3-shm"] { secure_fs::reject_special_file(&root.join(name))?; }
        if !db.try_exists().map_err(|e|e.to_string())? { drop(secure_fs::create_owned(&db)?); }
        let db_guard = secure_fs::Opened::source(&db,true)?;
        secure_fs::ensure_single_link(&db_guard.file)?;
        let mut connection = Connection::open(&db).map_err(db_error)?;
        connection.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        let app: i64 = connection.query_row("PRAGMA application_id",[],|r|r.get(0)).map_err(db_error)?;
        let version: i64 = connection.query_row("PRAGMA user_version",[],|r|r.get(0)).map_err(db_error)?;
        let tables: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[],|r|r.get(0)).map_err(db_error)?;
        if !((app == 0 && tables == 0 && version == 0) || (app == schema::APP_ID && [1,2,schema::VERSION].contains(&version))) {
            return Err("所选目录已有其他或不受支持版本数据库；不会迁移、重置或覆盖它".into());
        }
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL;").map_err(db_error)?;
        // 保留阶段2的 DB/journal inode 固定及 create_owned read/write 修复。
        let journal = root.join("vault.sqlite3-journal");
        if !journal.try_exists().map_err(|e|e.to_string())? { drop(secure_fs::create_owned(&journal)?); }
        let journal_guard = secure_fs::Opened::source(&journal,true)?;
        secure_fs::ensure_single_link(&journal_guard.file)?;
        let bundle = schema::prepare(&mut connection,app,version,tables,now_ms())?;
        connection.execute("UPDATE tasks SET state=CASE WHEN digest IS NOT NULL THEN 'staged' ELSE 'pending' END,
            next_attempt=?1,error='上次进程在归档中断；正在恢复' WHERE state IN ('copying','pending','staged')",[now_ms()]).map_err(db_error)?;
        let mut store = Self { root, configured_root, settings:bundle.capture.clone(), bundle, connection, startup_errors:Vec::new(),
            _root_guard:guard, _root_lock:lock, _db_guard:db_guard, _journal_guard:journal_guard };
        // 只重放用户已经提交的删除/恢复/清理意图；从不扫原文件或自动清档。
        store.startup_errors = crate::recovery::resume_all(&mut store);
        Ok(store)
    }
    pub fn validate(&self) -> Result<(),String> { self._root_guard.validate() }
    pub fn display_root(&self) -> Result<PathBuf,String> { self._root_guard.display_path() }
    pub fn config(&self) -> Result<ConfigBundle,String> { self.validate()?; Ok(self.bundle.clone()) }
    pub fn save_config(&mut self,bundle:ConfigBundle) -> Result<ConfigBundle,String> {
        self.validate()?;
        let bundle = config::validate_config(bundle)?;
        self.persist_config(bundle,false)
    }
    // 仅在线程已停止后，把设置和中断任务恢复一并提交。
    pub fn recover_and_save_config(&mut self,bundle:ConfigBundle) -> Result<ConfigBundle,String> {
        self.validate()?;
        self.persist_config(config::validate_config(bundle)?,true)
    }
    fn persist_config(&mut self,bundle:ConfigBundle,recover:bool) -> Result<ConfigBundle,String> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        schema::save_bundle(&tx,&bundle)?;
        if recover {recover_copying(&tx)?;}
        self._root_guard.validate()?;
        tx.commit().map_err(db_error)?;
        self.settings = bundle.capture.clone(); self.bundle = bundle.clone();
        Ok(bundle)
    }
    pub fn recover_copying(&self) -> Result<(),String> {
        self.validate()?;
        recover_copying(&self.connection)
    }
    pub fn favorite_save(&mut self,folder:FixedFolder) -> Result<(),String> {
        let mut bundle = self.bundle.clone();
        match bundle.favorites.iter().position(|f|f.id==folder.id) { Some(i)=>bundle.favorites[i]=folder, None=>bundle.favorites.push(folder) }
        self.save_config(bundle).map(|_|())
    }
    pub fn favorite_remove(&mut self,id:&str) -> Result<(),String> {
        let mut bundle = self.bundle.clone(); let before = bundle.favorites.len();
        bundle.favorites.retain(|f|f.id!=id);
        if before == bundle.favorites.len() { return Err("固定夹不存在，不会模拟移除成功".into()); }
        self.save_config(bundle).map(|_|())
    }
    pub fn counts(&self) -> Result<(u64,u64),String> {
        self.validate()?;
        self.connection.query_row("SELECT coalesce(sum(state IN ('pending','copying','staged')),0),coalesce(sum(state='failed'),0) FROM tasks",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)
    }
    pub fn recovery_errors(&self) -> Result<Vec<String>,String> {
        self.validate()?;
        let mut stmt = self.connection.prepare("SELECT entity_id||'：'||error FROM file_intents WHERE error IS NOT NULL ORDER BY created,entity_id LIMIT 64").map_err(db_error)?;
        let rows = stmt.query_map([],|r|r.get::<_,String>(0)).map_err(db_error)?;
        let mut errors = rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        errors.extend(self.startup_errors.iter().cloned()); errors.sort(); errors.dedup();
        Ok(errors)
    }

    fn prospective_snapshot(&self,date:&str,now:u64) -> Result<Option<Vec<CourseView>>,String> {
        timetable::parse_date(date)?;
        let existing: Option<bool> = self.connection.query_row("SELECT has_schedule FROM day_snapshots WHERE date=?1",[date],|r|r.get(0)).optional().map_err(db_error)?;
        if existing.is_none() || (existing==Some(false) && date>=event_date(now)?.as_str()) {
            let courses=timetable::snapshot(&self.bundle.timetable,date)?;
            if existing.is_none() || courses.iter().any(|c|c.slot_id.is_some()) {return Ok(Some(courses));}
        }
        Ok(None)
    }
    fn ensure_day(&mut self,date:&str) -> Result<(),String> {
        self.validate()?;
        let Some(courses)=self.prospective_snapshot(date,now_ms())? else {return Ok(());};
        self.materialize_day(date,courses)
    }
    fn materialize_day(&mut self,date:&str,courses:Vec<CourseView>) -> Result<(),String> {
        let has_schedule = courses.iter().any(|c|c.slot_id.is_some());
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        tx.execute("INSERT OR IGNORE INTO day_snapshots(date,created,has_schedule) VALUES(?1,?2,0)",params![date,now_ms()]).map_err(db_error)?;
        for course in courses {
            let ordinal = course.slot_id.as_ref().and_then(|id| self.bundle.timetable.slots.iter().filter(|s|s.is_class).position(|s|&s.id==id)).map(|n|n as i64+1).unwrap_or(10000);
            tx.execute("INSERT OR IGNORE INTO courses(id,date,slot_id,subject_id,subject_name,label,start_ms,end_ms,enabled,ordinal)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![course.id,course.date,course.slot_id,course.subject_id,course.subject_name,course.label,course.start_unix_ms,course.end_unix_ms,course.enabled,ordinal]).map_err(db_error)?;
        }
        if has_schedule { tx.execute("UPDATE day_snapshots SET has_schedule=1 WHERE date=?1",[date]).map_err(db_error)?; }
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    fn course_views(&self,date:&str) -> Result<Vec<CourseView>,String> {
        let mut stmt = self.connection.prepare("SELECT id,date,slot_id,subject_id,subject_name,label,start_ms,end_ms,enabled FROM courses WHERE date=?1 AND enabled=1 ORDER BY ordinal,start_ms,id").map_err(db_error)?;
        let rows = stmt.query_map([date],course_row).map_err(db_error)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(db_error)
    }
    pub fn courses(&mut self,date:&str) -> Result<Vec<CourseView>,String> {
        self.ensure_day(date)?;
        let mut courses = self.course_views(date)?;
        for item in self.items_for_date(date)? {
            let course_id = item.course_id.as_deref().ok_or("条目缺少课程关联")?;
            let course = courses.iter_mut().find(|c|c.id==course_id).ok_or("条目关联了缺失/禁用的课程快照")?;
            course.items.push(item);
        }
        Ok(courses)
    }
    pub fn change_course(&mut self,course_id:&str,subject_id:&str) -> Result<(),String> {
        self.validate()?;
        let course = self.connection.query_row("SELECT id,date,slot_id,subject_id,subject_name,label,start_ms,end_ms,enabled FROM courses WHERE id=?1",[course_id],course_row).map_err(db_error)?;
        if course.date != event_date(now_ms())? { return Err("卡片调课/取消只允许更改今天的课程，不修改历史或周课表".into()); }
        if course.slot_id.is_none() { return Err("未归类桶不能调课/取消".into()); }
        if subject_id.is_empty() {return self.cancel_course(&course);}
        let subject = self.bundle.timetable.subjects.iter().find(|s|s.id==subject_id).ok_or("调课科目不存在")?.clone();
        if !self.settings.excluded_subject_ids.is_empty() && course.subject_id.as_deref()!=Some(subject_id)
            && (!course.enabled || !course.subject_id.as_deref().is_some_and(|id|subject_allowed(&self.settings,id))
                || !subject_allowed(&self.settings,subject_id)) {
            self.guard_task_retarget(course_id)?;
        }
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        tx.execute("UPDATE courses SET subject_id=?2,subject_name=?3,enabled=1 WHERE id=?1",params![course_id,subject.id,subject.name]).map_err(db_error)?;
        tx.execute("UPDATE usages SET subject_name=?2 WHERE course_id=?1",params![course_id,subject.name]).map_err(db_error)?;
        tx.execute("UPDATE tasks SET subject_name=?2 WHERE course_id=?1",params![course_id,subject.name]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    fn guard_task_retarget(&self,course_id:&str) -> Result<(),String> {
        let waiting:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE course_id=?1 AND state<>'done')",[course_id],|r|r.get(0)).map_err(db_error)?;
        if waiting {return Err("该课次有待处理或可重试的捕获任务，不能通过调课改变其排除资格".into());}
        Ok(())
    }
    fn cancel_course(&mut self,course:&CourseView) -> Result<(),String> {
        if !self.settings.excluded_subject_ids.is_empty()
            && !course.subject_id.as_deref().is_some_and(|id|subject_allowed(&self.settings,id)) {
            self.guard_task_retarget(&course.id)?;
        }
        if !course.enabled {return Err("该当天课程已经取消".into());}
        let bucket=timetable::unclassified_id(&course.date);
        let active:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE course_id=?1 AND state IN ('pending','copying','staged'))",[&course.id],|r|r.get(0)).map_err(db_error)?;
        if active {return Err("本课次还有活动/待重试归档任务，须等待完成后取消，避免覆盖首次捕获意图".into());}
        let conflict:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks s JOIN tasks d ON d.course_id=?2 AND d.identity=s.identity WHERE s.course_id=?1)",params![course.id,bucket],|r|r.get(0)).map_err(db_error)?;
        if conflict {return Err("未归类已有同源身份任务，取消会冲突；未覆盖任务或丢弃历史，整个动作未提交".into());}
        let ids={let mut stmt=self.connection.prepare("SELECT entity_id FROM usages WHERE course_id=?1 ORDER BY entity_id").map_err(db_error)?;
            let rows=stmt.query_map([&course.id],|r|r.get::<_,String>(0)).map_err(db_error)?;rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?};
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        for id in ids {move_usage_tx(&tx,&id,&course.id,&bucket)?;}
        tx.execute("INSERT OR IGNORE INTO aliases(date,course_id,identity,source,display_name) SELECT date,?2,identity,source,display_name FROM aliases WHERE course_id=?1",params![course.id,bucket]).map_err(db_error)?;
        tx.execute("DELETE FROM aliases WHERE course_id=?1",[&course.id]).map_err(db_error)?;
        tx.execute("UPDATE tasks SET course_id=?2,subject_name='未归类' WHERE course_id=?1",params![course.id,bucket]).map_err(db_error)?;
        tx.execute("UPDATE courses SET enabled=0 WHERE id=?1",[&course.id]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }

    // 只读预判：不物化日期、不登记事件，不访问源文件。
    pub fn capture_allowed(&self,detected:u64) -> Result<bool,String> {
        self.validate()?;
        let date=event_date(detected)?;
        let courses=match self.prospective_snapshot(&date,now_ms())? {Some(courses)=>courses,None=>self.course_views(&date)?};
        Ok(capture_course_allowed(&self.settings,classified(&courses,&date,detected)?))
    }
    pub fn accept(&mut self,source:String,identity:Identity,size:u64,detected:u64) -> Result<bool,String> {
        self.validate()?;
        let date=event_date(detected)?;
        let prospective=self.prospective_snapshot(&date,now_ms())?;
        let course=match &prospective {
            Some(courses)=>classified(courses,&date,detected)?.clone(),
            None=>classified(&self.course_views(&date)?,&date,detected)?.clone(),
        };
        if !capture_course_allowed(&self.settings,&course) {return Ok(false);}
        if let Some(courses)=prospective {self.materialize_day(&date,courses)?;}
        let course_id=course.id;
        let subject_name=course.subject_name;
        let identity = serde_json::to_string(&identity).map_err(|e|e.to_string())?;
        let name = source.rsplit(['\\','/']).next().unwrap_or(&source).to_owned();
        let extension = name.rsplit_once('.').map(|(_,e)|e.to_lowercase()).unwrap_or_default();
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE course_id=?1 AND identity=?2)",params![course_id,identity],|r|r.get(0)).map_err(db_error)?;
        if !exists && self.counts()?.0>=MAX_PENDING { return Err("捕获任务队列已满（1024），未接受新任务".into()); }
        let ordinal: i64 = self.connection.query_row("SELECT ordinal FROM courses WHERE id=?1",[&course_id],|r|r.get(0)).map_err(db_error)?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        // 高频同课次同源合并；不同课次单独保存任务及使用，别名不丢弃。
        tx.execute("INSERT OR IGNORE INTO aliases(date,course_id,identity,source,display_name) VALUES(?1,?2,?3,?4,?5)",params![date,course_id,identity,source,name]).map_err(db_error)?;
        if exists {
            // 同课次乱序事件保留更早的真实时间，不延长重试、不重启失败任务，也不改用户显示名。
            tx.execute("UPDATE tasks SET detected=min(detected,?3) WHERE course_id=?1 AND identity=?2",params![course_id,identity,detected]).map_err(db_error)?;
            tx.execute("UPDATE usages SET
                task_id=CASE WHEN ?3<detected THEN (SELECT id FROM tasks WHERE course_id=?1 AND identity=?2) ELSE task_id END,
                display_name=CASE WHEN ?3<detected THEN (SELECT display_name FROM tasks WHERE course_id=?1 AND identity=?2) ELSE display_name END,
                detected=min(detected,?3) WHERE course_id=?1 AND entity_id=(SELECT entity_id FROM tasks WHERE course_id=?1 AND identity=?2)",params![course_id,identity,detected]).map_err(db_error)?;
            self._root_guard.validate()?;tx.commit().map_err(db_error)?;return Ok(false);
        }
        let id = Uuid::new_v4().to_string(); let entity_id = Uuid::new_v4().to_string();
        let stem = secure_fs::safe_filename(name.rsplit_once('.').map(|(s,_)|s).unwrap_or(&name));
        let suffix = if extension.is_empty() {String::new()} else {format!(".{extension}")};
        let folder = if course.slot_id.is_none() {"未归类".into()} else {format!("{ordinal:02}-{}",secure_fs::safe_filename(&subject_name))};
        let final_rel = format!("archive/{date}/{folder}/{stem}--{entity_id}{suffix}");
        let stage_rel = format!("staging/{id}.part"); let now = now_ms();
        tx.execute("INSERT INTO tasks(id,source,identity,display_name,extension,detected,date,course_id,subject_name,state,deadline,next_attempt,stage_rel,final_rel,entity_id,size,attribution_known)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,?11,?12,?13,?14,?15,1)",params![id,source,identity,name,extension,detected,date,course_id,subject_name,now.saturating_add(RETRY_WINDOW_MS),now,stage_rel,final_rel,entity_id,size]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)?;
        Ok(true)
    }
    pub fn due(&mut self) -> Result<Option<Task>,String> {
        self.validate()?;
        let id = due_id(&self.connection,&self.settings,now_ms())?;
        let Some(id)=id else {return Ok(None);};
        let task = self.task(&id)?;
        self.connection.execute("UPDATE tasks SET state='copying',attempts=attempts+1 WHERE id=?1 AND state IN ('pending','staged')",[&id]).map_err(db_error)?;
        Ok(Some(Task {attempts:task.attempts+1,..task}))
    }
    pub fn task(&self,id:&str) -> Result<Task,String> {
        self.validate()?;
        self.connection.query_row("SELECT id,source,identity,display_name,extension,detected,date,course_id,subject_name,deadline,attempts,state,stage_rel,final_rel,entity_id,digest,size FROM tasks WHERE id=?1",[id],|r| {
            Ok(Task {id:r.get(0)?,source:r.get(1)?,identity:json_identity(r,2)?,display_name:r.get(3)?,extension:r.get(4)?,detected:r.get(5)?,date:r.get(6)?,course_id:r.get(7)?,subject_name:r.get(8)?,deadline:r.get(9)?,attempts:r.get(10)?,state:r.get(11)?,stage_rel:r.get(12)?,final_rel:r.get(13)?,entity_id:r.get(14)?,digest:r.get(15)?,size:r.get(16)?})
        }).map_err(db_error)
    }
    pub fn require_attempt(&self,task:&Task) -> Result<(),String> {
        self.validate()?;
        require_attempt(&self.connection,&self.settings,task)
    }
    pub fn staged(&self,task:&Task,digest:&str,size:u64) -> Result<(),String> {
        self.require_attempt(task)?;
        self.connection.execute("UPDATE tasks SET state='staged',digest=?2,size=?3,error=NULL WHERE id=?1 AND state='copying'",params![task.id,digest,size]).map_err(db_error)?;
        Ok(())
    }
    pub fn by_content(&self,digest:&str,size:u64) -> Result<Option<Entity>,String> {
        self.validate()?;
        self.connection.query_row(&format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE digest=?1 AND size=?2 AND state<>'purged'"),params![digest,size],entity_row).optional().map_err(db_error)
    }
    pub fn entity(&self,id:&str) -> Result<Entity,String> {
        self.validate()?;
        self.connection.query_row(&format!("SELECT {ENTITY_COLUMNS} FROM entities WHERE id=?1"),[id],entity_row).map_err(db_error)
    }
    pub fn pin_entity_identity(&self,entity:&Entity,identity:&Identity) -> Result<(),String> {
        self.validate()?;
        if let Some(expected)=&entity.owned_identity {
            if expected!=identity {return Err("归档文件身份已被外部替换，拒绝接纳同名文件".into());}
            let available:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM entities WHERE id=?1 AND rel=?2 AND digest=?3 AND size=?4 AND state='available')",params![entity.id,entity.rel,entity.digest,entity.size],|r|r.get(0)).map_err(db_error)?;
            if !available {return Err("实体状态在验证期间改变，未将失效资料交出租约".into());}return Ok(());
        }
        let json = serde_json::to_string(identity).map_err(|e|e.to_string())?;
        let updated = self.connection.execute("UPDATE entities SET owned_identity=?2 WHERE id=?1 AND rel=?3 AND digest=?4 AND size=?5 AND state='available' AND (owned_identity IS NULL OR owned_identity=?2)",params![entity.id,json,entity.rel,entity.digest,entity.size]).map_err(db_error)?;
        if updated!=1 { return Err("实体状态/身份在固定期间变化，未覆盖元数据".into()); }
        Ok(())
    }
    pub fn finish(&mut self,task:&Task,entity:&Entity,insert:bool) -> Result<(),String> {
        self.validate()?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        require_attempt(&tx,&self.settings,task)?;
        // 复制/重试晚于乱序事件或当天调课时，读取当前 task 的首次时间与快照，不能用旧 clone 覆盖。
        let (course_id,date,subject_name,detected,display_name,extension): (String,String,String,u64,String,String) = tx.query_row(
            "SELECT t.course_id,t.date,c.subject_name,t.detected,t.display_name,t.extension FROM tasks t JOIN courses c ON c.id=t.course_id WHERE t.id=?1 AND t.state IN ('copying','staged')",
            [&task.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(db_error)?;
        if !insert {
            let available: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM entities WHERE id=?1 AND state='available' AND digest=?2 AND size=?3 AND NOT EXISTS(SELECT 1 FROM file_intents WHERE entity_id=?1))",params![entity.id,entity.digest,entity.size],|r|r.get(0)).map_err(db_error)?;
            if !available { return Err("去重实体在提交前已失效/进入回收操作，拒绝建立成功使用记录".into()); }
        } else {
            let identity = serde_json::to_string(entity.owned_identity.as_ref().ok_or("新实体缺少已验证自有文件身份")?).map_err(|e|e.to_string())?;
            tx.execute("INSERT INTO entities(id,rel,digest,size,state,display_name,extension,archived,owned_identity) VALUES(?1,?2,?3,?4,'available',?5,?6,?7,?8)",params![entity.id,entity.rel,entity.digest,entity.size,display_name,extension,entity.archived,identity]).map_err(db_error)?;
        }
        tx.execute("INSERT INTO usages(course_id,entity_id,date,subject_name,detected,task_id,display_name) VALUES(?1,?2,?3,?4,?5,?6,?7)
            ON CONFLICT(course_id,entity_id) DO UPDATE SET subject_name=excluded.subject_name,
            task_id=CASE WHEN excluded.detected<usages.detected THEN excluded.task_id ELSE usages.task_id END,
            display_name=CASE WHEN excluded.detected<usages.detected THEN excluded.display_name ELSE usages.display_name END,
            detected=min(usages.detected,excluded.detected)",params![course_id,entity.id,date,subject_name,detected,task.id,display_name]).map_err(db_error)?;
        tx.execute("UPDATE tasks SET state='done',entity_id=?2,digest=?3,size=?4,subject_name=?5,error=NULL WHERE id=?1 AND state IN ('copying','staged')",params![task.id,entity.id,entity.digest,entity.size,subject_name]).map_err(db_error)?;
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    pub fn invalidate(&mut self,id:&str,error:&str) -> Result<(),String> {
        self.validate()?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let updated = tx.execute("UPDATE entities SET state='failed',error=?2 WHERE id=?1 AND state IN ('available','failed') AND NOT EXISTS(SELECT 1 FROM file_intents WHERE entity_id=?1)",params![id,error]).map_err(db_error)?;
        // 删除/恢复/清理并发状态不能被旧校验结果覆盖成 failed。
        if updated>0 { tx.execute("UPDATE tasks SET state='failed',error=?2 WHERE entity_id=?1 AND state='done'",params![id,error]).map_err(db_error)?; }
        self._root_guard.validate()?; tx.commit().map_err(db_error)
    }
    pub fn fail(&self,task:&Task,error:&str,terminal:bool) -> Result<(),String> {
        self.validate()?;
        if !attempt_allowed(&self.connection,&self.settings,&task.id,Some(task.attempts))? {return Ok(());}
        let now=now_ms(); let failed=terminal || now>=task.deadline;
        let seconds = match task.attempts {0..=2=>2,3..=5=>5,6..=8=>15,9..=12=>30,13..=18=>60,_=>120};
        let next=now.saturating_add(seconds*1000).min(task.deadline);
        self.connection.execute("UPDATE tasks SET state=CASE WHEN ?2 THEN 'failed' WHEN digest IS NOT NULL THEN 'staged' ELSE 'pending' END,next_attempt=?3,error=?4 WHERE id=?1 AND state IN ('copying','staged') AND attempts=?5",params![task.id,failed,next,error,task.attempts]).map_err(db_error)?;
        Ok(())
    }
    pub fn retry(&self,id:&str) -> Result<(),String> {
        self.validate()?;
        if !attempt_allowed(&self.connection,&self.settings,id,None)? {return Err("科目被排除、归属不明确或任务不是失败状态，禁止重试".into());}
        let task=self.task(id)?;
        if task.state!="failed" {return Err("只能手动重试失败的捕获任务".into());}
        if self.counts()?.0>=MAX_PENDING {return Err("捕获任务队列已满".into());}
        let blocked: bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM entities WHERE id=?1 AND state NOT IN ('available','failed'))",[&task.entity_id],|r|r.get(0)).map_err(db_error)?;
        if blocked {return Err("关联实体处于回收/已清理状态；请先恢复，不自动重新启用或覆盖".into());}
        let now=now_ms();
        // attempts 同时是尝试代际，手动重试不能归零而重新接纳旧 worker 的 fail/finish。
        self.connection.execute("UPDATE tasks SET state=CASE WHEN digest IS NULL THEN 'pending' ELSE 'staged' END,deadline=?2,next_attempt=?3,error=NULL WHERE id=?1 AND state='failed'",params![id,now.saturating_add(RETRY_WINDOW_MS),now]).map_err(db_error)?;
        Ok(())
    }
    /// 撤下失败记录，不删除来源、归档或无法验证的暂存文件。
    pub fn dismiss_failed(&mut self,id:&str) -> Result<(),String> {
        self.validate()?;
        let task=self.task(id)?;
        if task.state!="failed" {return Err("只能移除仍处于失败状态的记录；任务可能已经重试".into());}
        if self.has_active_entity(&task.entity_id)? || self.intent(&task.entity_id)?.is_some() {return Err("该资料有活动任务或回收操作，不能移除失败记录".into());}
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let changed=tx.execute("UPDATE tasks SET state='dismissed' WHERE state='failed' AND
            (id=?1 OR (entity_id=?2 AND EXISTS(SELECT 1 FROM entities WHERE id=?2 AND state='failed')))",params![id,task.entity_id]).map_err(db_error)?;
        if changed==0 {return Err("失败记录已经变化，未移除".into());}
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }
    fn items_for_date(&self,date:&str) -> Result<Vec<ArchiveItem>,String> {
        self.validate()?;
        let mut stmt=self.connection.prepare("SELECT e.id,e.display_name,e.extension,e.size,u.detected,e.state,
            CASE WHEN e.state='failed' THEN u.task_id ELSE NULL END,e.error,u.course_id,u.subject_name
            FROM usages u JOIN entities e ON e.id=u.entity_id WHERE u.date=?1 AND e.state IN ('available','failed')
                AND (e.state='available' OR NOT EXISTS(SELECT 1 FROM tasks dismissed WHERE dismissed.id=u.task_id AND dismissed.state='dismissed'))
            UNION ALL SELECT t.id,t.display_name,t.extension,t.size,t.detected,CASE WHEN t.state='failed' THEN 'failed' ELSE 'pending' END,t.id,t.error,t.course_id,c.subject_name
            FROM tasks t JOIN courses c ON c.id=t.course_id WHERE t.date=?1 AND t.state IN ('pending','copying','staged','failed')
                AND NOT EXISTS(SELECT 1 FROM usages u WHERE u.entity_id=t.entity_id AND u.course_id=t.course_id)
                AND NOT EXISTS(SELECT 1 FROM entities e WHERE e.id=t.entity_id AND e.state NOT IN ('available','failed')) ORDER BY 5,9,1").map_err(db_error)?;
        let rows=stmt.query_map([date],item_row).map_err(db_error)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(db_error)
    }
    pub fn today(&mut self) -> Result<Vec<ArchiveItem>,String> { let date=event_date(now_ms())?;self.ensure_day(&date)?;self.items_for_date(&date) }

    pub fn library(&self,query:LibraryQuery) -> Result<LibraryPage,String> {
        self.validate()?; let query=crate::library::validate_library_query(query)?;
        let matcher=NameMatcher::new(&query.name,query.regex)?;
        self.connection.create_scalar_function("cv_name_match",1,rusqlite::functions::FunctionFlags::SQLITE_UTF8|rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            move |context| {let name=context.get::<String>(0)?;Ok(matcher.matches(&name))}).map_err(db_error)?;
        let (key,label)=match query.group_by.as_str() {
            "none"=>("''","''"), "date"=>("u.date","u.date"), "month"=>("substr(u.date,1,7)","substr(u.date,1,7)"),
            "subject"=>("coalesce(c.subject_id,'unclassified')||':'||u.subject_name","u.subject_name"),
            "course"=>("u.course_id","u.date||' · '||c.label||' · '||u.subject_name"),
            "type"=>("e.extension","CASE WHEN e.extension='' THEN '无扩展名' ELSE upper(e.extension) END"), _=>unreachable!(),
        };
        let mut filter=String::from("e.state IN ('available','failed') AND (e.state='available' OR NOT EXISTS(SELECT 1 FROM tasks dismissed WHERE dismissed.id=u.task_id AND dismissed.state='dismissed'))"); let mut values:Vec<Value>=Vec::new();
        if let Some(date)=query.date_from {filter.push_str(" AND u.date>=?");values.push(date.into());}
        if let Some(date)=query.date_to {filter.push_str(" AND u.date<=?");values.push(date.into());}
        if !query.subjects.is_empty() {
            let marks=vec!["?";query.subjects.len()].join(",");
            filter.push_str(&format!(" AND (coalesce(c.subject_id,'unclassified') IN ({marks}) OR u.subject_name IN ({marks}))"));
            values.extend(query.subjects.iter().cloned().map(Value::from));values.extend(query.subjects.into_iter().map(Value::from));
        }
        if !query.extensions.is_empty() {filter.push_str(&format!(" AND e.extension IN ({})",vec!["?";query.extensions.len()].join(",")));values.extend(query.extensions.into_iter().map(Value::from));}
        if !query.name.is_empty() {
            filter.push_str(" AND (cv_name_match(e.display_name) OR cv_name_match(u.display_name) OR EXISTS(
                SELECT 1 FROM tasks a_t JOIN aliases a ON a.course_id=a_t.course_id AND a.identity=a_t.identity
                WHERE a_t.entity_id=e.id AND cv_name_match(a.display_name)))");
        }
        let base=format!("WITH candidates AS (SELECT e.id,e.display_name,e.extension,e.size,u.detected,e.state,
            CASE WHEN e.state='failed' THEN u.task_id ELSE NULL END AS task_id,e.error,u.course_id,u.subject_name,
            e.archived,{key} AS group_key,{label} AS group_label,
            row_number() OVER(PARTITION BY e.id,{key} ORDER BY u.detected DESC,u.course_id,u.task_id) AS rn
            FROM entities e JOIN usages u ON u.entity_id=e.id JOIN courses c ON c.id=u.course_id WHERE {filter})");
        let total:u64=self.connection.query_row(&format!("{base} SELECT count(*) FROM candidates WHERE rn=1"),params_from_iter(values.iter()),|r|r.get(0)).map_err(db_error)?;
        let sort=match query.sort_by.as_str() {"archived"=>"archived","used"=>"detected","name"=>"display_name COLLATE NOCASE","size"=>"size",_=>unreachable!()};
        let direction=if query.descending {"DESC"} else {"ASC"};
        let sql=format!("{base} SELECT id,display_name,extension,size,detected,state,task_id,error,course_id,subject_name,group_key,group_label
            FROM candidates WHERE rn=1 ORDER BY {sort} {direction},id ASC,group_key ASC LIMIT ? OFFSET ?");
        values.push(i64::from(query.limit).into());values.push((query.offset as i64).into());
        let mut stmt=self.connection.prepare(&sql).map_err(db_error)?;
        let rows=stmt.query_map(params_from_iter(values.iter()),|r|Ok(LibraryRow {group_key:r.get(10)?,group_label:r.get(11)?,item:item_row(r)?})).map_err(db_error)?;
        Ok(LibraryPage {rows:rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?,total})
    }
    pub fn detail(&self,id:&str) -> Result<ArchiveDetail,String> {
        let entity=self.entity(id)?;
        let mut stmt=self.connection.prepare("SELECT date,course_id,subject_name,detected FROM usages WHERE entity_id=?1 ORDER BY detected,course_id").map_err(db_error)?;
        let rows=stmt.query_map([id],|r|Ok(UsageDetail {date:r.get(0)?,course_id:r.get(1)?,subject_name:r.get(2)?,detected_unix_ms:r.get(3)?})).map_err(db_error)?;
        let uses=rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?;drop(stmt);
        let mut stmt=self.connection.prepare("SELECT t.source FROM tasks t WHERE t.entity_id=?1 UNION
            SELECT a.source FROM tasks t JOIN aliases a ON a.course_id=t.course_id AND a.identity=t.identity WHERE t.entity_id=?1 ORDER BY 1").map_err(db_error)?;
        let rows=stmt.query_map([id],|r|r.get::<_,String>(0)).map_err(db_error)?;
        let source_paths=rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        let usage=uses.last();
        let task_id=if entity.state=="failed" {self.connection.query_row("SELECT id FROM tasks WHERE entity_id=?1 AND state='failed' ORDER BY detected,id LIMIT 1",[id],|r|r.get(0)).optional().map_err(db_error)?} else {None};
        let deleted=!matches!(entity.state.as_str(),"available"|"failed");
        let mut rel=if deleted {entity.trash_rel.clone().unwrap_or_else(||entity.rel.clone())} else {entity.rel.clone()};
        if let Some(intent)=self.intent(id)? {
            let from=secure_fs::relative(&self.root,&intent.from_rel)?;
            if from.try_exists().map_err(|e|format!("读取回收副本当前位置失败：{e}"))? {rel=intent.from_rel;}
            else if let Some(to)=intent.to_rel {
                if secure_fs::relative(&self.root,&to)?.try_exists().map_err(|e|format!("读取回收目标位置失败：{e}"))? {rel=to;}
            }
        }
        secure_fs::relative(&self.root,&rel)?;
        let archive_path=self.display_root()?.join(&rel).to_string_lossy().into_owned();
        let item=ArchiveItem {id:entity.id,display_name:entity.display_name,extension:entity.extension,size:entity.size,
            detected_unix_ms:usage.map(|u|u.detected_unix_ms).unwrap_or(entity.archived),state:entity.state,task_id,error:entity.error,
            course_id:usage.map(|u|u.course_id.clone()),subject_name:usage.map(|u|u.subject_name.clone())};
        Ok(ArchiveDetail {item,archive_path,source_paths,uses,deleted})
    }
    pub fn edit_archive(&mut self,id:&str,name:&str,course_id:Option<&str>) -> Result<(),String> {
        self.validate()?;let entity=self.entity(id)?;
        if !matches!(entity.state.as_str(),"available"|"failed") {return Err("回收/已清理实体不能编辑".into());}
        config::path_component(name)?;
        if !entity.extension.is_empty() && !name.rsplit_once('.').is_some_and(|(_,ext)|ext.eq_ignore_ascii_case(&entity.extension)) {
            return Err(format!("显示名称须保留真实格式 .{} 扩展名",entity.extension));
        }
        if self.has_active_entity(id)? {return Err("资料正在归档/恢复，暂不能改关联".into());}
        let from=if course_id.is_some() {Some(self.connection.query_row("SELECT course_id FROM usages WHERE entity_id=?1 ORDER BY detected DESC,course_id DESC LIMIT 1",[id],|r|r.get::<_,String>(0)).map_err(db_error)?)} else {None};
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        if let (Some(from),Some(to))=(from.as_deref(),course_id) {move_usage_tx(&tx,id,from,to)?;}
        tx.execute("UPDATE entities SET display_name=?2 WHERE id=?1",params![id,name]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }
    pub fn move_usage(&mut self,id:&str,from:&str,to:&str) -> Result<(),String> {
        self.validate()?;let entity=self.entity(id)?;
        if !matches!(entity.state.as_str(),"available"|"failed") || self.has_active_entity(id)? {return Err("实体处于回收/归档状态，不能纠正使用记录".into());}
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        move_usage_tx(&tx,id,from,to)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }
    pub fn trash_list(&self) -> Result<Vec<ArchiveDetail>,String> {
        self.validate()?;
        let ids={let mut stmt=self.connection.prepare("SELECT id FROM entities WHERE state IN ('deleting','deleted','restoring','purging') ORDER BY deleted_at DESC,id").map_err(db_error)?;
            let rows=stmt.query_map([],|r|r.get::<_,String>(0)).map_err(db_error)?;rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?};
        ids.into_iter().map(|id|self.detail(&id)).collect()
    }
    pub fn has_active_entity(&self,id:&str) -> Result<bool,String> {
        self.validate()?;
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE entity_id=?1 AND state IN ('pending','copying','staged'))",[id],|r|r.get(0)).map_err(db_error)
    }
    pub fn busy_volumes(&self) -> Result<Vec<String>,String> {
        self.validate()?;
        let mut result=vec![secure_fs::volume_prefix(&self.root)?];
        let mut stmt=self.connection.prepare("SELECT DISTINCT identity FROM tasks WHERE state IN ('pending','copying','staged')").map_err(db_error)?;
        let rows=stmt.query_map([],|r|json_identity(r,0)).map_err(db_error)?;
        for identity in rows {result.push(config::volume_id(&identity.map_err(db_error)?.volume_name)?);}
        result.sort();result.dedup();Ok(result)
    }

    pub(crate) fn intent(&self,id:&str) -> Result<Option<FileIntent>,String> {
        self.validate()?;
        self.connection.query_row("SELECT entity_id,kind,from_rel,to_rel,identity,digest,size FROM file_intents WHERE entity_id=?1",[id],intent_row).optional().map_err(db_error)
    }
    pub(crate) fn intents(&self) -> Result<Vec<FileIntent>,String> {
        self.validate()?;
        let mut stmt=self.connection.prepare("SELECT entity_id,kind,from_rel,to_rel,identity,digest,size FROM file_intents ORDER BY created,entity_id").map_err(db_error)?;
        let rows=stmt.query_map([],intent_row).map_err(db_error)?;rows.collect::<Result<Vec<_>,_>>().map_err(db_error)
    }
    pub(crate) fn begin_intent(&mut self,entity:&Entity,kind:&str,from_rel:&str,to_rel:Option<&str>,identity:&Identity) -> Result<FileIntent,String> {
        self.validate()?;
        secure_fs::relative(&self.root,from_rel)?;if let Some(to)=to_rel {secure_fs::relative(&self.root,to)?;}
        if self.has_active_entity(&entity.id)? {return Err("资料有活动归档任务，不能回收/恢复/清理".into());}
        if entity.owned_identity.as_ref().is_some_and(|i|i!=identity) {return Err("自有文件身份已被替换，禁止删除/移动替代文件".into());}
        let pending=match kind {"trash"=>"deleting","restore"=>"restoring","purge"=>"purging",_=>return Err("未知回收操作".into())};
        let valid=match kind {"trash"=>matches!(entity.state.as_str(),"available"|"failed"),_=>entity.state=="deleted"};
        if !valid {return Err("实体状态不允许此回收操作".into());}
        let json=serde_json::to_string(identity).map_err(|e|e.to_string())?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let changed=tx.execute("UPDATE entities SET state=?2,owned_identity=?3,error=NULL,
            trash_rel=CASE WHEN ?4='trash' THEN ?5 ELSE trash_rel END,
            deleted_at=CASE WHEN ?4='trash' THEN ?6 ELSE deleted_at END WHERE id=?1 AND state=?7 AND digest=?8 AND size=?9
            AND NOT EXISTS(SELECT 1 FROM file_intents WHERE entity_id=?1)",params![entity.id,pending,json,kind,to_rel,now_ms(),entity.state,entity.digest,entity.size]).map_err(db_error)?;
        if changed!=1 {return Err("实体回收状态并发变化，未覆盖已有动作".into());}
        tx.execute("INSERT INTO file_intents(entity_id,kind,from_rel,to_rel,identity,digest,size,created) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![entity.id,kind,from_rel,to_rel,json,entity.digest,entity.size,now_ms()]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)?;
        Ok(FileIntent {entity_id:entity.id.clone(),kind:kind.into(),from_rel:from_rel.into(),to_rel:to_rel.map(str::to_owned),identity:identity.clone(),digest:entity.digest.clone(),size:entity.size})
    }
    pub(crate) fn complete_intent(&mut self,intent:&FileIntent) -> Result<(),String> {
        self.validate()?;
        let (before,after)=match intent.kind.as_str() {"trash"=>("deleting","deleted"),"restore"=>("restoring","available"),"purge"=>("purging","purged"),_=>return Err("未知恢复意图".into())};
        let json=serde_json::to_string(&intent.identity).map_err(|e|e.to_string())?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM file_intents WHERE entity_id=?1 AND kind=?2 AND identity=?3 AND digest=?4 AND size=?5)",params![intent.entity_id,intent.kind,json,intent.digest,intent.size],|r|r.get(0)).map_err(db_error)?;
        if !exists {return Err("回收意图已改变，拒绝提交旧动作".into());}
        let updated=tx.execute("UPDATE entities SET state=?2,error=NULL,trash_rel=CASE WHEN ?2='available' THEN NULL ELSE trash_rel END,
            deleted_at=CASE WHEN ?2='available' THEN NULL ELSE deleted_at END WHERE id=?1 AND state=?3 AND owned_identity=?4",params![intent.entity_id,after,before,json]).map_err(db_error)?;
        if updated!=1 {return Err("回收实体状态/自有身份已改变，未覆盖它".into());}
        if after=="available" {
            tx.execute("UPDATE tasks SET state='done',error=NULL WHERE entity_id=?1 AND state='failed' AND digest=?2 AND size=?3
                AND EXISTS(SELECT 1 FROM usages u WHERE u.entity_id=tasks.entity_id AND u.course_id=tasks.course_id)",params![intent.entity_id,intent.digest,intent.size]).map_err(db_error)?;
        }
        tx.execute("DELETE FROM file_intents WHERE entity_id=?1",[&intent.entity_id]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)?;
        self.startup_errors.retain(|error|!error.contains(&intent.entity_id));
        Ok(())
    }
    pub(crate) fn intent_error(&self,id:&str,error:&str) -> Result<(),String> {
        self.validate()?;
        let message:String=error.chars().take(2000).collect();
        let tx=self.connection.unchecked_transaction().map_err(db_error)?;
        tx.execute("UPDATE file_intents SET error=?2 WHERE entity_id=?1",params![id,message]).map_err(db_error)?;
        tx.execute("UPDATE entities SET error=?2 WHERE id=?1 AND EXISTS(SELECT 1 FROM file_intents WHERE entity_id=?1)",params![id,message]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }
    pub(crate) fn thumbnail_record(&self,id:&str) -> Result<Option<ThumbnailRecord>,String> {
        self.validate()?;
        self.connection.query_row("SELECT entity_id,rel,source_digest,png_digest,identity,size,created,used FROM thumbnails WHERE entity_id=?1",[id],thumbnail_row).optional().map_err(db_error)
    }
    pub(crate) fn thumbnails(&self) -> Result<Vec<ThumbnailRecord>,String> {
        self.validate()?;
        let mut stmt=self.connection.prepare("SELECT entity_id,rel,source_digest,png_digest,identity,size,created,used FROM thumbnails ORDER BY used,entity_id").map_err(db_error)?;
        let rows=stmt.query_map([],thumbnail_row).map_err(db_error)?;rows.collect::<Result<Vec<_>,_>>().map_err(db_error)
    }
    pub(crate) fn save_thumbnail(&mut self,record:&ThumbnailRecord) -> Result<(),String> {
        self.validate()?;
        let identity=serde_json::to_string(&record.identity).map_err(|e|e.to_string())?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM entities WHERE id=?1 AND state='available' AND digest=?2)",params![record.entity_id,record.source_digest],|r|r.get(0)).map_err(db_error)?;
        if !valid {return Err("生成缩略图期间实体状态变化，未写缓存索引".into());}
        tx.execute("INSERT INTO thumbnails(entity_id,rel,source_digest,png_digest,identity,size,created,used) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![record.entity_id,record.rel,record.source_digest,record.png_digest,identity,record.size,record.created,record.used]).map_err(db_error)?;
        self._root_guard.validate()?;tx.commit().map_err(db_error)
    }
    pub(crate) fn touch_thumbnail(&self,id:&str) -> Result<(),String> {self.validate()?;self.connection.execute("UPDATE thumbnails SET used=?2 WHERE entity_id=?1",params![id,now_ms()]).map_err(db_error)?;Ok(())}
    pub(crate) fn drop_thumbnail_record(&self,record:&ThumbnailRecord) -> Result<(),String> {
        self.validate()?;
        let json=serde_json::to_string(&record.identity).map_err(|e|e.to_string())?;
        let changed=self.connection.execute("DELETE FROM thumbnails WHERE entity_id=?1 AND rel=?2 AND identity=?3",params![record.entity_id,record.rel,json]).map_err(db_error)?;
        if changed!=1 {return Err("缓存身份记录变化，未删除替代记录".into());}Ok(())
    }
}

fn classified<'a>(courses:&'a [CourseView],date:&str,detected:u64) -> Result<&'a CourseView,String> {
    let bucket=timetable::unclassified_id(date);
    let id=timetable::classify_course(courses,date,detected).unwrap_or(bucket.as_str());
    courses.iter().find(|c|c.id==id && c.enabled && c.date==date).ok_or("首次事件时间没有可用的当日快照".into())
}
fn subject_allowed(settings:&CaptureSettings,id:&str) -> bool {
    config::identifier(id,"科目 ID").is_ok() && !settings.excluded_subject_ids.iter().any(|s|s==id)
}
fn capture_course_allowed(settings:&CaptureSettings,course:&CourseView) -> bool {
    settings.excluded_subject_ids.is_empty() || match &course.subject_id {
        Some(id)=>subject_allowed(settings,id),
        None=>course.slot_id.is_none(),
    }
}
fn recover_copying(connection:&Connection) -> Result<(),String> {
    connection.execute("UPDATE tasks SET state=CASE WHEN digest IS NULL THEN 'pending' ELSE 'staged' END,
        next_attempt=?1,error='后台中断后恢复原捕获意图' WHERE state='copying'",[now_ms()]).map_err(db_error)?;
    Ok(())
}
// 同日改课后仍读当前关联，不用 task clone 的显示名或旧归属。
const TASK_POLICY_FIELDS:&str="t.id,c.id,c.subject_id,c.slot_id,t.date,t.attribution_known FROM tasks t
    LEFT JOIN courses c ON c.id=t.course_id AND c.date=t.date AND c.enabled=1";
fn task_policy_allows(settings:&CaptureSettings,row:&rusqlite::Row<'_>) -> rusqlite::Result<bool> {
    if settings.excluded_subject_ids.is_empty() {return Ok(true);}
    let course:Option<String>=row.get(1)?;
    let Some(course)=course else {return Ok(false);};
    let subject:Option<String>=row.get(2)?;
    let slot:Option<String>=row.get(3)?;
    if let Some(id)=subject {return Ok(slot.is_some() && subject_allowed(settings,&id));}
    let date:String=row.get(4)?;
    let known:bool=row.get(5)?;
    Ok(known && slot.is_none() && course==timetable::unclassified_id(&date))
}
fn due_id(connection:&Connection,settings:&CaptureSettings,now:u64) -> Result<Option<String>,String> {
    let mut stmt=connection.prepare_cached(&format!("SELECT {TASK_POLICY_FIELDS}
        WHERE t.state IN ('pending','staged') AND t.next_attempt<=?1 ORDER BY t.next_attempt,t.detected,t.id")).map_err(db_error)?;
    let mut rows=stmt.query([now]).map_err(db_error)?;
    while let Some(row)=rows.next().map_err(db_error)? {
        if task_policy_allows(settings,row).map_err(db_error)? {return row.get(0).map(Some).map_err(db_error);}
    }
    Ok(None)
}
fn attempt_allowed(connection:&Connection,settings:&CaptureSettings,id:&str,attempt:Option<u64>) -> Result<bool,String> {
    let mut stmt=connection.prepare_cached(&format!("SELECT {TASK_POLICY_FIELDS} WHERE t.id=?1 AND
        ((?2 IS NULL AND t.state='failed') OR (?2=t.attempts AND t.state IN ('copying','staged')))" )).map_err(db_error)?;
    stmt.query_row(params![id,attempt],|row|task_policy_allows(settings,row))
        .optional().map(|allowed|allowed.unwrap_or(false)).map_err(db_error)
}

fn require_attempt(connection:&Connection,settings:&CaptureSettings,task:&Task) -> Result<(),String> {
    if !attempt_allowed(connection,settings,&task.id,Some(task.attempts))? {
        return Err("归档任务非当前尝试、科目被排除或归属无法确认".into());
    }
    Ok(())
}
fn json_identity(row:&rusqlite::Row<'_>,index:usize) -> rusqlite::Result<Identity> {
    let json:String=row.get(index)?;
    serde_json::from_str(&json).map_err(|e|rusqlite::Error::FromSqlConversionFailure(index,Type::Text,Box::new(e)))
}
fn entity_row(r:&rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    let identity:Option<String>=r.get(11)?;
    let identity=identity.map(|json|serde_json::from_str(&json)).transpose().map_err(|e|rusqlite::Error::FromSqlConversionFailure(11,Type::Text,Box::new(e)))?;
    Ok(Entity {id:r.get(0)?,rel:r.get(1)?,digest:r.get(2)?,size:r.get(3)?,state:r.get(4)?,display_name:r.get(5)?,extension:r.get(6)?,archived:r.get(7)?,error:r.get(8)?,trash_rel:r.get(9)?,deleted_at:r.get(10)?,owned_identity:identity})
}
fn course_row(r:&rusqlite::Row<'_>) -> rusqlite::Result<CourseView> {
    Ok(CourseView {id:r.get(0)?,date:r.get(1)?,slot_id:r.get(2)?,subject_id:r.get(3)?,subject_name:r.get(4)?,label:r.get(5)?,start_unix_ms:r.get(6)?,end_unix_ms:r.get(7)?,enabled:r.get(8)?,items:Vec::new()})
}
fn item_row(r:&rusqlite::Row<'_>) -> rusqlite::Result<ArchiveItem> {
    Ok(ArchiveItem {id:r.get(0)?,display_name:r.get(1)?,extension:r.get(2)?,size:r.get(3)?,detected_unix_ms:r.get(4)?,state:r.get(5)?,task_id:r.get(6)?,error:r.get(7)?,course_id:r.get(8)?,subject_name:r.get(9)?})
}
fn intent_row(r:&rusqlite::Row<'_>) -> rusqlite::Result<FileIntent> {
    Ok(FileIntent {entity_id:r.get(0)?,kind:r.get(1)?,from_rel:r.get(2)?,to_rel:r.get(3)?,identity:json_identity(r,4)?,digest:r.get(5)?,size:r.get(6)?})
}
fn thumbnail_row(r:&rusqlite::Row<'_>) -> rusqlite::Result<ThumbnailRecord> {
    Ok(ThumbnailRecord {entity_id:r.get(0)?,rel:r.get(1)?,source_digest:r.get(2)?,png_digest:r.get(3)?,identity:json_identity(r,4)?,size:r.get(5)?,created:r.get(6)?,used:r.get(7)?})
}
#[cfg(test)]
pub(crate) mod exclusion_tests {
    use super::*;
    pub(crate) struct Fixture {pub(crate) store:Option<Store>,path:PathBuf}
    impl Fixture {
        pub(crate) fn new()->Self {
            let path=std::env::temp_dir().join(format!("celesvault-policy-{}",Uuid::new_v4()));
            let store=Store::open(path.to_string_lossy().into_owned()).unwrap();
            Self {store:Some(store),path}
        }
        fn store(&mut self)->&mut Store {self.store.as_mut().unwrap()}
    }
    impl Drop for Fixture {
        fn drop(&mut self) {drop(self.store.take());let _=std::fs::remove_dir_all(&self.path);}
    }
    pub(crate) fn setup(store:&mut Store)->Vec<CourseView> {
        let mut bundle=ConfigBundle::default();
        bundle.timetable.subjects=vec![Subject {id:"excluded".into(),name:"排除科目".into(),color:None},Subject {id:"allowed".into(),name:"允许科目".into(),color:None}];
        bundle.timetable.slots=vec![
            TimeSlot {id:"first".into(),label:"第一节".into(),start:"08:00".into(),end:"09:00".into(),is_class:true},
            TimeSlot {id:"second".into(),label:"第二节".into(),start:"10:00".into(),end:"11:00".into(),is_class:true}];
        for weekday in 1..=7 {
            bundle.timetable.lessons.push(WeekLesson {weekday,slot_id:"first".into(),subject_id:"excluded".into(),enabled:true});
            bundle.timetable.lessons.push(WeekLesson {weekday,slot_id:"second".into(),subject_id:"allowed".into(),enabled:true});
        }
        store.save_config(bundle).unwrap();
        timetable::snapshot(&store.bundle.timetable,&event_date(now_ms()).unwrap()).unwrap()
    }
    fn identity(n:u8)->Identity {Identity {volume_serial:1,volume_name:"test-volume".into(),file_id:[n;16],created:1}}
    fn accept(store:&mut Store,n:u8,time:u64)->bool {
        store.accept(format!("C:\\不存在的源目录\\{n}.docx"),identity(n),10,time).unwrap()
    }
    fn scalar(store:&Store,sql:&str)->u64 {store.connection.query_row(sql,[],|r|r.get(0)).unwrap()}
    fn exclude(store:&mut Store,enabled:bool) {
        let mut bundle=store.bundle.clone();
        bundle.capture.excluded_subject_ids=if enabled {vec!["excluded".into()]} else {vec![]};
        store.save_config(bundle).unwrap();
    }
    #[test]
    fn rejection_is_read_only_and_keeps_current_next_course_boundaries() {
        let mut f=Fixture::new();let s=f.store();let courses=setup(s);exclude(s,true);
        let first=&courses[0];let second=&courses[1];
        for time in [first.start_unix_ms-1,first.start_unix_ms,first.end_unix_ms-1] {
            assert!(!s.capture_allowed(time).unwrap());assert!(!accept(s,1,time));
        }
        assert_eq!(scalar(s,"SELECT count(*) FROM day_snapshots"),0);
        for table in ["courses","tasks","aliases","usages","entities"] {assert_eq!(scalar(s,&format!("SELECT count(*) FROM {table}")),0);}
        // 课间归下一课，而不是删掉排除课后重新分类。
        assert!(s.capture_allowed(first.end_unix_ms).unwrap());
        assert!(s.capture_allowed(second.end_unix_ms).unwrap());
        let mut last_excluded=s.bundle.clone();last_excluded.capture.excluded_subject_ids=vec!["allowed".into()];
        s.save_config(last_excluded).unwrap();
        assert!(!s.capture_allowed(first.end_unix_ms).unwrap());
        assert!(!s.capture_allowed(second.end_unix_ms-1).unwrap());
        assert!(s.capture_allowed(second.end_unix_ms).unwrap());
        exclude(s,true);
        assert!(accept(s,2,second.start_unix_ms));
        assert_eq!(s.task(&due_id(&s.connection,&s.settings,now_ms()).unwrap().unwrap()).unwrap().subject_name,"允许科目");
        // 显示名称不是准入身份；模拟历史快照同名/改名。
        s.connection.execute("UPDATE courses SET subject_name='同名科目'",[]).unwrap();
        assert!(!s.capture_allowed(first.start_unix_ms).unwrap());
        assert!(s.capture_allowed(second.start_unix_ms).unwrap());
        let mut bundle=s.bundle.clone();bundle.timetable=Timetable::default();s.save_config(bundle).unwrap();
        // 已物化日快照优先于后改周课表。
        assert!(!s.capture_allowed(first.start_unix_ms).unwrap());
        let tomorrow=timetable::parse_date(&first.date).unwrap().succ_opt().unwrap().to_string();
        let bucket=timetable::unclassified(&tomorrow).unwrap();
        assert!(s.capture_allowed(bucket.start_unix_ms).unwrap());
        assert_eq!(scalar(s,"SELECT count(*) FROM day_snapshots"),1);
    }
    #[test]
    fn excluded_queue_retry_recovery_and_stale_attempt_cannot_run() {
        let mut f=Fixture::new();let s=f.store();let c=setup(s);
        assert!(accept(s,1,c[0].start_unix_ms));
        let old=s.due().unwrap().unwrap();
        exclude(s,true);
        assert!(s.require_attempt(&old).is_err());
        assert!(s.staged(&old,"digest",10).is_err());
        s.fail(&old,"旧错误",false).unwrap();
        assert_eq!(s.task(&old.id).unwrap().state,"copying");
        let entity=Entity {id:old.entity_id.clone(),rel:old.final_rel.clone(),digest:"digest".into(),size:10,state:"available".into(),display_name:"资料.docx".into(),extension:"docx".into(),archived:now_ms(),error:None,trash_rel:None,deleted_at:None,owned_identity:Some(identity(9))};
        assert!(s.finish(&old,&entity,true).is_err());
        assert!(s.finish(&old,&entity,false).is_err());
        s.recover_copying().unwrap();
        assert!(s.due().unwrap().is_none());
        assert_eq!(s.task(&old.id).unwrap().attempts,old.attempts);
        // 被阻止的早任务不能饿死后面的允许任务。
        assert!(accept(s,2,c[1].start_unix_ms));
        let allowed=s.due().unwrap().unwrap();assert_ne!(allowed.id,old.id);
        s.connection.execute("UPDATE tasks SET state='failed' WHERE id=?1",[&old.id]).unwrap();
        assert!(s.retry(&old.id).is_err());
        exclude(s,false);s.retry(&old.id).unwrap();
        // 旧 fail 不能把手动重试后的 pending 任务改回可执行状态。
        s.fail(&old,"陈旧错误",true).unwrap();assert_eq!(s.task(&old.id).unwrap().state,"pending");
        let current=s.due().unwrap().unwrap();assert_eq!(current.id,old.id);
        assert!(s.require_attempt(&old).is_err());
        assert!(s.require_attempt(&current).is_ok());
    }
    #[test]
    fn completed_history_survives_and_blocked_dedup_cannot_add_a_usage() {
        let mut f=Fixture::new();let s=f.store();let c=setup(s);
        assert!(accept(s,1,c[0].start_unix_ms));let done=s.due().unwrap().unwrap();
        s.staged(&done,"digest",10).unwrap();
        let entity=Entity {id:done.entity_id.clone(),rel:done.final_rel.clone(),digest:"digest".into(),size:10,state:"available".into(),display_name:"历史.docx".into(),extension:"docx".into(),archived:now_ms(),error:None,trash_rel:None,deleted_at:None,owned_identity:Some(identity(9))};
        s.finish(&done,&entity,true).unwrap();
        assert!(accept(s,2,c[0].start_unix_ms+1));let waiting=s.due().unwrap().unwrap();
        exclude(s,true);
        assert!(!accept(s,1,c[0].start_unix_ms-1));
        assert!(s.finish(&waiting,&entity,false).is_err());
        assert_eq!(s.task(&done.id).unwrap().state,"done");
        assert_eq!(s.entity(&entity.id).unwrap().state,"available");
        assert_eq!(scalar(s,"SELECT count(*) FROM usages"),1);
        assert_eq!(scalar(s,"SELECT count(*) FROM aliases"),2);
        exclude(s,false);s.finish(&waiting,&entity,false).unwrap();
        assert_eq!(scalar(s,"SELECT count(*) FROM usages"),1);
    }
    #[test]
    fn unknown_old_bucket_is_blocked_but_new_genuine_unclassified_is_allowed() {
        let mut f=Fixture::new();let s=f.store();let c=setup(s);
        assert!(accept(s,1,c[1].end_unix_ms));let task=s.due().unwrap().unwrap();
        s.staged(&task,"digest",10).unwrap();
        s.connection.execute("UPDATE tasks SET attribution_known=0 WHERE id=?1",[&task.id]).unwrap();
        exclude(s,true);assert!(s.due().unwrap().is_none());assert!(s.require_attempt(&task).is_err());
        assert!(accept(s,2,c[1].end_unix_ms));let genuine=s.due().unwrap().unwrap();assert_ne!(genuine.id,task.id);
        exclude(s,false);assert_eq!(s.due().unwrap().unwrap().id,task.id);
    }
    #[test]
    fn dismissing_failed_capture_hides_record_without_deleting_files_or_reactivating_task() {
        let mut fixture=Fixture::new();let store=fixture.store();let courses=setup(store);
        assert!(accept(store,1,courses[0].start_unix_ms));
        let task=store.due().unwrap().unwrap();
        assert!(store.dismiss_failed(&task.id).is_err());
        store.fail(&task,"测试失败",true).unwrap();
        let stage=secure_fs::relative(&store.root,&task.stage_rel).unwrap();
        std::fs::write(&stage,b"unverified stage remains").unwrap();
        store.dismiss_failed(&task.id).unwrap();
        assert_eq!(store.task(&task.id).unwrap().state,"dismissed");
        assert!(store.items_for_date(&courses[0].date).unwrap().is_empty());
        assert_eq!(store.counts().unwrap(),(0,0));
        assert!(store.retry(&task.id).is_err());
        store.recover_copying().unwrap();assert!(store.due().unwrap().is_none());
        assert!(!accept(store,1,courses[0].start_unix_ms));
        assert_eq!(store.task(&task.id).unwrap().state,"dismissed");
        assert_eq!(std::fs::read(&stage).unwrap(),b"unverified stage remains");
        assert_eq!(scalar(store,"SELECT count(*) FROM aliases"),1);
        assert_eq!(scalar(store,"SELECT count(*) FROM tasks"),1);
    }
    #[test]
    fn dismissed_failed_archive_keeps_history_but_is_removed_from_today_and_library() {
        let mut fixture=Fixture::new();let store=fixture.store();let courses=setup(store);
        assert!(accept(store,1,courses[0].start_unix_ms));let task=store.due().unwrap().unwrap();
        store.staged(&task,"digest",10).unwrap();
        let entity=Entity {id:task.entity_id.clone(),rel:task.final_rel.clone(),digest:"digest".into(),size:10,state:"available".into(),display_name:"失败资料.docx".into(),extension:"docx".into(),archived:now_ms(),error:None,trash_rel:None,deleted_at:None,owned_identity:Some(identity(9))};
        store.finish(&task,&entity,true).unwrap();
        assert!(store.dismiss_failed(&task.id).is_err());
        store.invalidate(&entity.id,"校验失败").unwrap();
        assert_eq!(store.items_for_date(&courses[0].date).unwrap().len(),1);
        store.dismiss_failed(&task.id).unwrap();
        assert!(store.items_for_date(&courses[0].date).unwrap().is_empty());
        assert_eq!(store.library(LibraryQuery::default()).unwrap().total,0);
        assert_eq!(store.entity(&entity.id).unwrap().state,"failed");
        assert_eq!(scalar(store,"SELECT count(*) FROM usages"),1);
        assert_eq!(scalar(store,"SELECT count(*) FROM entities"),1);
        assert!(store.trash_list().unwrap().is_empty());
    }
    #[test]
    fn same_day_retarget_cannot_allow_waiting_or_exclude_active_tasks() {
        let mut f=Fixture::new();let s=f.store();let c=setup(s);
        assert!(accept(s,1,c[0].start_unix_ms));let task=s.due().unwrap().unwrap();exclude(s,true);
        for state in ["copying","pending","staged","failed"] {
            s.connection.execute("UPDATE tasks SET state=?2 WHERE id=?1",params![task.id,state]).unwrap();
            assert!(s.change_course(&c[0].id,"allowed").is_err());
            assert!(s.change_course(&c[0].id,"").is_err());
        }
        assert!(accept(s,2,c[1].start_unix_ms));let allowed=s.due().unwrap().unwrap();
        assert!(s.change_course(&c[1].id,"excluded").is_err());
        s.connection.execute("UPDATE tasks SET state='done' WHERE id IN (?1,?2)",params![task.id,allowed.id]).unwrap();
        s.change_course(&c[0].id,"allowed").unwrap();
        assert_eq!(scalar(s,"SELECT count(*) FROM tasks WHERE state='done'"),2);
    }
}

fn move_usage_tx(tx:&rusqlite::Transaction<'_>,id:&str,from:&str,to:&str) -> Result<(),String> {
    let (date,detected,task_id,name):(String,u64,String,String)=tx.query_row("SELECT date,detected,task_id,display_name FROM usages WHERE entity_id=?1 AND course_id=?2",params![id,from],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db_error)?;
    let (target_date,subject,enabled):(String,String,bool)=tx.query_row("SELECT date,subject_name,enabled FROM courses WHERE id=?1",[to],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
    if !enabled || date!=target_date {return Err("纠正归属须选择同日期的启用课次，不跨日期改历史".into());}
    if from==to {return Ok(());}
    let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks s JOIN tasks d ON d.course_id=?3 AND d.identity=s.identity WHERE s.entity_id=?1 AND s.course_id=?2 AND d.id<>s.id)",params![id,from,to],|r|r.get(0)).map_err(db_error)?;
    if conflict {return Err("目标课次已有同源身份任务，不能通过覆盖/删除任务历史强行合并".into());}
    tx.execute("INSERT OR IGNORE INTO aliases(date,course_id,identity,source,display_name) SELECT date,?3,identity,source,display_name FROM aliases a
        WHERE course_id=?2 AND EXISTS(SELECT 1 FROM tasks t WHERE t.entity_id=?1 AND t.course_id=?2 AND t.identity=a.identity)",params![id,from,to]).map_err(db_error)?;
    tx.execute("DELETE FROM aliases AS a WHERE course_id=?2 AND EXISTS(SELECT 1 FROM tasks t WHERE t.entity_id=?1 AND t.course_id=?2 AND t.identity=a.identity)",params![id,from]).map_err(db_error)?;
    tx.execute("UPDATE tasks SET course_id=?3,subject_name=?4 WHERE entity_id=?1 AND course_id=?2",params![id,from,to,subject]).map_err(db_error)?;
    tx.execute("INSERT INTO usages(course_id,entity_id,date,subject_name,detected,task_id,display_name) VALUES(?1,?2,?3,?4,?5,?6,?7)
        ON CONFLICT(course_id,entity_id) DO UPDATE SET subject_name=excluded.subject_name,
        task_id=CASE WHEN excluded.detected<usages.detected THEN excluded.task_id ELSE usages.task_id END,
        display_name=CASE WHEN excluded.detected<usages.detected THEN excluded.display_name ELSE usages.display_name END,
        detected=min(usages.detected,excluded.detected)",params![to,id,date,subject,detected,task_id,name]).map_err(db_error)?;
    tx.execute("DELETE FROM usages WHERE entity_id=?1 AND course_id=?2",params![id,from]).map_err(db_error)?;
    Ok(())
}
