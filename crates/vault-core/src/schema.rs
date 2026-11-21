//! SQLite v1/v2 -> v3：保留 ID、源身份、任务意图、实体路径和历史，标记可证明的归属。
use crate::{api::ConfigBundle, config, timetable};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

pub(crate) const APP_ID: i64 = 0x43564C54;
pub(crate) const VERSION: i64 = 3;
pub(crate) fn db_error(error: rusqlite::Error) -> String { format!("持久数据库操作失败：{error}") }

const COURSES_SQL: &str = "
CREATE TABLE day_snapshots(date TEXT PRIMARY KEY, created INTEGER NOT NULL, has_schedule INTEGER NOT NULL);
CREATE TABLE courses(id TEXT PRIMARY KEY, date TEXT NOT NULL REFERENCES day_snapshots(date),
    slot_id TEXT, subject_id TEXT, subject_name TEXT NOT NULL, label TEXT NOT NULL,
    start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, enabled INTEGER NOT NULL, ordinal INTEGER NOT NULL,
    UNIQUE(date,slot_id));
CREATE INDEX courses_date ON courses(date,ordinal,id);
";
const AUX_SQL: &str = "
CREATE TABLE file_intents(entity_id TEXT PRIMARY KEY REFERENCES entities(id), kind TEXT NOT NULL,
    from_rel TEXT NOT NULL, to_rel TEXT, identity TEXT NOT NULL, digest TEXT NOT NULL,
    size INTEGER NOT NULL, created INTEGER NOT NULL, error TEXT);
CREATE TABLE thumbnails(entity_id TEXT PRIMARY KEY REFERENCES entities(id), rel TEXT NOT NULL UNIQUE,
    source_digest TEXT NOT NULL, png_digest TEXT NOT NULL, identity TEXT NOT NULL, size INTEGER NOT NULL,
    created INTEGER NOT NULL, used INTEGER NOT NULL);
CREATE INDEX thumbnails_lru ON thumbnails(used,entity_id);
";
const INDEX_SQL: &str = "
CREATE UNIQUE INDEX entity_content ON entities(digest,size) WHERE state<>'purged';
CREATE INDEX entities_state_archive ON entities(state,archived,id);
CREATE INDEX tasks_due ON tasks(state,next_attempt,detected,id);
CREATE INDEX tasks_entity ON tasks(entity_id,course_id);
CREATE INDEX usages_entity ON usages(entity_id,detected,course_id);
CREATE INDEX usages_date_course ON usages(date,course_id,entity_id);
";
const TASKS_SQL: &str = "
CREATE TABLE tasks(id TEXT PRIMARY KEY, source TEXT NOT NULL, identity TEXT NOT NULL,
    display_name TEXT NOT NULL, extension TEXT NOT NULL, detected INTEGER NOT NULL,
    date TEXT NOT NULL, course_id TEXT NOT NULL REFERENCES courses(id), subject_name TEXT NOT NULL,
    state TEXT NOT NULL, deadline INTEGER NOT NULL, next_attempt INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0, stage_rel TEXT NOT NULL UNIQUE, final_rel TEXT NOT NULL UNIQUE,
    attribution_known INTEGER NOT NULL DEFAULT 0,
    entity_id TEXT NOT NULL, digest TEXT, size INTEGER NOT NULL, error TEXT,
    UNIQUE(course_id,identity));
CREATE TABLE usages(course_id TEXT NOT NULL REFERENCES courses(id), entity_id TEXT NOT NULL REFERENCES entities(id),
    date TEXT NOT NULL, subject_name TEXT NOT NULL, detected INTEGER NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id), display_name TEXT NOT NULL, PRIMARY KEY(course_id,entity_id));
CREATE TABLE aliases(date TEXT NOT NULL, course_id TEXT NOT NULL REFERENCES courses(id), identity TEXT NOT NULL,
    source TEXT NOT NULL, display_name TEXT NOT NULL, PRIMARY KEY(course_id,identity,source,display_name));
";

pub(crate) fn save_bundle(tx: &Transaction<'_>, bundle: &ConfigBundle) -> Result<(), String> {
    for (key, value) in [("bundle", serde_json::to_string(bundle).map_err(|e| e.to_string())?),
        ("capture", serde_json::to_string(&bundle.capture).map_err(|e| e.to_string())?)] {
        tx.execute("INSERT INTO config(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(db_error)?;
    }
    Ok(())
}

pub(crate) fn prepare(connection: &mut Connection, app: i64, version: i64, tables: i64, now: u64) -> Result<ConfigBundle, String> {
    if app == 0 && tables == 0 && version == 0 {
        let bundle = config::validate_config(ConfigBundle::default())?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        tx.execute_batch("CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE entities(id TEXT PRIMARY KEY,digest TEXT NOT NULL,size INTEGER NOT NULL,
            rel TEXT NOT NULL UNIQUE,state TEXT NOT NULL,error TEXT,display_name TEXT NOT NULL,
            extension TEXT NOT NULL,archived INTEGER NOT NULL,owned_identity TEXT,trash_rel TEXT,deleted_at INTEGER);").map_err(db_error)?;
        tx.execute_batch(COURSES_SQL).map_err(db_error)?;
        tx.execute_batch(TASKS_SQL).map_err(db_error)?;
        tx.execute_batch(AUX_SQL).map_err(db_error)?;
        tx.execute_batch(INDEX_SQL).map_err(db_error)?;
        save_bundle(&tx,&bundle)?;
        tx.execute_batch(&format!("PRAGMA application_id={APP_ID}; PRAGMA user_version={VERSION};")).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        return Ok(bundle);
    }
    if app != APP_ID || ![1, 2, VERSION].contains(&version) { return Err("数据库不属于 CelesVault 或结构版本不受支持；不会重置/覆盖".into()); }
    if version == 1 { return migrate_v1(connection,now); }
    let json: String = connection.query_row("SELECT value FROM config WHERE key='bundle'",[],|r|r.get(0)).map_err(db_error)?;
    let bundle = config::parse_config(&json).map_err(|e| format!("持久配置损坏：{e}"))?;
    let capture_json: String = connection.query_row("SELECT value FROM config WHERE key='capture'",[],|r|r.get(0)).map_err(db_error)?;
    let capture = config::validate_capture(serde_json::from_str(&capture_json).map_err(|e|format!("监测设置损坏：{e}"))?)?;
    if bundle.capture != capture { return Err("完整配置与监测镜像不一致；不会静默选择其中一份".into()); }
    if version == 2 {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        add_attribution(&tx)?;
        tx.commit().map_err(db_error)?;
    }
    Ok(bundle)
}

fn add_attribution(tx:&Transaction<'_>) -> Result<(),String> {
    // 旧未归类任务可能从 v1 迁入，不能把它们猜成真实的无课归属。
    tx.execute_batch(&format!("ALTER TABLE tasks ADD COLUMN attribution_known INTEGER NOT NULL DEFAULT 0;
        PRAGMA user_version={VERSION};")).map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_bundle_and_capture_mirror_default_together_and_mismatch_is_rejected() {
        let mut db=Connection::open_in_memory().unwrap();
        let bundle=prepare(&mut db,0,0,0,0).unwrap();
        let mut old=serde_json::to_value(&bundle).unwrap();
        old["capture"].as_object_mut().unwrap().remove("excludedSubjectIds");
        let old_capture=old["capture"].to_string();
        db.execute("UPDATE config SET value=?1 WHERE key='bundle'",[old.to_string()]).unwrap();
        db.execute("UPDATE config SET value=?1 WHERE key='capture'",[old_capture]).unwrap();
        // 模拟旧 v2 数据库，升级新增的保守归属标记不能改动任务意图。
        db.execute_batch("ALTER TABLE tasks DROP COLUMN attribution_known; PRAGMA user_version=2;").unwrap();
        assert!(prepare(&mut db,APP_ID,2,1,0).unwrap().capture.excluded_subject_ids.is_empty());
        let mut capture=bundle.capture;
        capture.excluded_subject_ids=vec!["removed".into()];
        db.execute("UPDATE config SET value=?1 WHERE key='capture'",[serde_json::to_string(&capture).unwrap()]).unwrap();
        assert!(prepare(&mut db,APP_ID,VERSION,1,0).is_err());
    }
}

fn migrate_v1(connection: &mut Connection, now: u64) -> Result<ConfigBundle, String> {
    let capture_json: Option<String> = connection.query_row("SELECT value FROM config WHERE key='capture'",[],|r|r.get(0)).optional().map_err(db_error)?;
    let mut bundle = ConfigBundle::default();
    if let Some(json) = capture_json { bundle.capture = serde_json::from_str(&json).map_err(|e|format!("迁移前监测配置损坏：{e}"))?; }
    let bundle = config::validate_config(bundle)?;
    let days = {
        let mut stmt = connection.prepare("SELECT date FROM tasks UNION SELECT date FROM usages UNION SELECT date FROM aliases ORDER BY date").map_err(db_error)?;
        let rows = stmt.query_map([],|r|r.get::<_,String>(0)).map_err(db_error)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(db_error)?
    };
    let snapshots = days.iter().map(|date|timetable::unclassified(date)).collect::<Result<Vec<_>,_>>()?;
    // SQLite 的表重建须在事务外关 FK；任何失败均 rollback，随后重新开启 FK。
    connection.execute_batch("PRAGMA foreign_keys=OFF;").map_err(db_error)?;
    let result = (|| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(db_error)?;
        let old_counts: (i64,i64,i64) = tx.query_row("SELECT (SELECT count(*) FROM tasks),(SELECT count(*) FROM usages),(SELECT count(*) FROM aliases)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
        tx.execute_batch(COURSES_SQL).map_err(db_error)?;
        for course in snapshots {
            tx.execute("INSERT INTO day_snapshots(date,created,has_schedule) VALUES(?1,?2,0)",params![course.date,now]).map_err(db_error)?;
            tx.execute("INSERT INTO courses(id,date,slot_id,subject_id,subject_name,label,start_ms,end_ms,enabled,ordinal)
                VALUES(?1,?2,NULL,NULL,?3,?4,?5,?6,1,10000)",params![course.id,course.date,course.subject_name,course.label,course.start_unix_ms,course.end_unix_ms]).map_err(db_error)?;
        }
        tx.execute_batch("ALTER TABLE entities ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
            ALTER TABLE entities ADD COLUMN extension TEXT NOT NULL DEFAULT '';
            ALTER TABLE entities ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE entities ADD COLUMN owned_identity TEXT;
            ALTER TABLE entities ADD COLUMN trash_rel TEXT;
            ALTER TABLE entities ADD COLUMN deleted_at INTEGER;
            UPDATE entities SET
                display_name=coalesce((SELECT u.display_name FROM usages u WHERE u.entity_id=entities.id ORDER BY u.detected,u.task_id LIMIT 1),
                    (SELECT t.display_name FROM tasks t WHERE t.entity_id=entities.id ORDER BY t.detected,t.id LIMIT 1),id),
                extension=coalesce((SELECT t.extension FROM tasks t WHERE t.entity_id=entities.id ORDER BY t.detected,t.id LIMIT 1),''),
                archived=coalesce((SELECT min(u.detected) FROM usages u WHERE u.entity_id=entities.id),
                    (SELECT min(t.detected) FROM tasks t WHERE t.entity_id=entities.id),0);
            CREATE TABLE tasks_new(id TEXT PRIMARY KEY,source TEXT NOT NULL,identity TEXT NOT NULL,display_name TEXT NOT NULL,
                extension TEXT NOT NULL,detected INTEGER NOT NULL,date TEXT NOT NULL,course_id TEXT NOT NULL REFERENCES courses(id),
                subject_name TEXT NOT NULL,state TEXT NOT NULL,deadline INTEGER NOT NULL,next_attempt INTEGER NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,stage_rel TEXT NOT NULL UNIQUE,final_rel TEXT NOT NULL UNIQUE,
                entity_id TEXT NOT NULL,digest TEXT,size INTEGER NOT NULL,error TEXT,UNIQUE(course_id,identity));
            INSERT INTO tasks_new SELECT id,source,identity,display_name,extension,detected,date,'unclassified:'||date,'未归类',
                state,deadline,next_attempt,attempts,stage_rel,final_rel,entity_id,digest,size,error FROM tasks;
            CREATE TABLE usages_new(course_id TEXT NOT NULL REFERENCES courses(id),entity_id TEXT NOT NULL REFERENCES entities(id),
                date TEXT NOT NULL,subject_name TEXT NOT NULL,detected INTEGER NOT NULL,task_id TEXT NOT NULL REFERENCES tasks(id),
                display_name TEXT NOT NULL,PRIMARY KEY(course_id,entity_id));
            INSERT INTO usages_new SELECT 'unclassified:'||date,entity_id,date,'未归类',detected,task_id,display_name FROM usages;
            CREATE TABLE aliases_new(date TEXT NOT NULL,course_id TEXT NOT NULL REFERENCES courses(id),identity TEXT NOT NULL,
                source TEXT NOT NULL,display_name TEXT NOT NULL,PRIMARY KEY(course_id,identity,source,display_name));
            INSERT INTO aliases_new SELECT date,'unclassified:'||date,identity,source,display_name FROM aliases;
            DROP TABLE usages; DROP TABLE aliases; DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks; ALTER TABLE usages_new RENAME TO usages; ALTER TABLE aliases_new RENAME TO aliases;
            DROP INDEX entity_content;").map_err(db_error)?;
        tx.execute_batch(AUX_SQL).map_err(db_error)?;
        tx.execute_batch(INDEX_SQL).map_err(db_error)?;
        let counts: (i64,i64,i64) = tx.query_row("SELECT (SELECT count(*) FROM tasks),(SELECT count(*) FROM usages),(SELECT count(*) FROM aliases)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_error)?;
        if counts != old_counts { return Err("迁移行数不一致，已拒绝提交，不会丢弃历史".into()); }
        let violations: i64 = tx.query_row("SELECT count(*) FROM pragma_foreign_key_check",[],|r|r.get(0)).map_err(db_error)?;
        if violations != 0 { return Err("迁移外键关联不一致，已拒绝提交".into()); }
        save_bundle(&tx,&bundle)?;
        add_attribution(&tx)?;
        tx.commit().map_err(db_error)
    })();
    let reenable = connection.execute_batch("PRAGMA foreign_keys=ON;").map_err(db_error);
    match (result,reenable) {
        (Ok(()),Ok(())) => Ok(bundle),
        (Err(error),Ok(())) => Err(error),
        (result,Err(error)) => Err(format!("{}；恢复 SQLite 外键开关失败：{error}",result.err().unwrap_or_else(||"迁移已提交".into()))),
    }
}
