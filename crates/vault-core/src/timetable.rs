use crate::api::{CourseView, Subject, TimeSlot, Timetable, WeekLesson};
use chrono::{Datelike, Local, NaiveDate, TimeZone};
use std::collections::{HashMap, HashSet};

pub const MAX_SUBJECTS: usize = 256;
pub const MAX_SLOTS: usize = 128;
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const CSV_HEADER: [&str; 10] = ["kind", "id", "name", "start", "end", "isClass", "weekday", "slotId", "subjectId", "enabled"];

pub fn parse_date(value: &str) -> Result<NaiveDate, String> {
    if value.len() != 10 || value.as_bytes()[4] != b'-' || value.as_bytes()[7] != b'-'
        || !value.bytes().enumerate().all(|(i, b)| [4, 7].contains(&i) || b.is_ascii_digit()) {
        return Err("日期必须是 YYYY-MM-DD".into());
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "日期不存在")?;
    if !(1970..=9999).contains(&date.year()) { return Err("日期年份须在 1970–9999 范围内".into()); }
    Ok(date)
}

pub fn time_minutes(value: &str) -> Result<u32, String> {
    let b = value.as_bytes();
    if b.len() != 5 || b[2] != b':' || ![b[0], b[1], b[3], b[4]].iter().all(u8::is_ascii_digit) {
        return Err("时间必须是 HH:mm，不支持跨午夜时间段".into());
    }
    let hour = u32::from(b[0] - b'0') * 10 + u32::from(b[1] - b'0');
    let minute = u32::from(b[3] - b'0') * 10 + u32::from(b[4] - b'0');
    if hour > 23 || minute > 59 { return Err("时间超出 00:00–23:59 范围".into()); }
    Ok(hour * 60 + minute)
}

pub fn validate_timetable(mut timetable: Timetable) -> Result<Timetable, String> {
    if timetable.subjects.len() > MAX_SUBJECTS || timetable.slots.len() > MAX_SLOTS || timetable.lessons.len() > 7 * MAX_SLOTS {
        return Err("课表超过 256 科目、128 时间段或 896 星期课程上限".into());
    }
    let mut subjects = HashSet::new(); let mut names = HashSet::new();
    for subject in &mut timetable.subjects {
        crate::config::identifier(&subject.id, "科目 ID")?;
        crate::config::text(&subject.name, "科目名称", 128, false)?;
        subject.name = subject.name.trim().to_owned();
        if !subjects.insert(subject.id.clone()) || !names.insert(subject.name.to_lowercase()) { return Err("科目 ID 或名称重复".into()); }
    }
    let mut slots = HashMap::new(); let mut previous_end = 0;
    for (index, slot) in timetable.slots.iter_mut().enumerate() {
        crate::config::identifier(&slot.id, "时间段 ID")?;
        crate::config::text(&slot.label, "时间段名称", 128, false)?;
        slot.label = slot.label.trim().to_owned();
        let start = time_minutes(&slot.start)?; let end = time_minutes(&slot.end)?;
        if start >= end { return Err(format!("时间段“{}”结束时间须晚于开始时间", slot.label)); }
        if index > 0 && start < previous_end { return Err("每日时间段须按开始时间排列且不能重叠（包含非课程段）".into()); }
        previous_end = end;
        if slots.insert(slot.id.clone(), (index, slot.is_class)).is_some() { return Err("时间段 ID 重复".into()); }
    }
    let mut cells = HashSet::new();
    for lesson in &timetable.lessons {
        if !(1..=7).contains(&lesson.weekday) { return Err("星期须使用 ISO 1–7（周一至周日）".into()); }
        let (_, is_class) = slots.get(&lesson.slot_id).ok_or("星期课程引用了不存在的时间段")?;
        if !is_class { return Err("非课程时间段不能设置星期课程".into()); }
        if !cells.insert((lesson.weekday, lesson.slot_id.clone())) { return Err("同一星期/时间段不能重复配置课程".into()); }
        if lesson.subject_id.is_empty() {
            if lesson.enabled { return Err("启用的课程必须选择科目".into()); }
        } else {
            crate::config::identifier(&lesson.subject_id, "课程科目 ID")?;
            if !subjects.contains(&lesson.subject_id) { return Err("星期课程引用了不存在的科目".into()); }
        }
    }
    timetable.lessons.sort_by_key(|l| (l.weekday, slots.get(&l.slot_id).map(|v| v.0).unwrap_or(usize::MAX)));
    Ok(timetable)
}

pub fn event_date(ms: u64) -> Result<String, String> {
    let ms = i64::try_from(ms).map_err(|_| "事件时间溢出")?;
    let time = Local.timestamp_millis_opt(ms).single().ok_or("事件时间无效")?;
    let date = time.format("%Y-%m-%d").to_string();
    parse_date(&date)?;
    Ok(date)
}

fn local_ms(date: NaiveDate, minutes: u32) -> Result<u64, String> {
    let naive = date.and_hms_opt(minutes / 60, minutes % 60, 0).ok_or("本地课程时间无效")?;
    let value = Local.from_local_datetime(&naive).single().ok_or("本地课程时间处于夏令时缺口或歧义，不能猜测归属")?.timestamp_millis();
    u64::try_from(value).map_err(|_| "课程时间早于可支持的 Unix 时间范围".into())
}

pub fn unclassified_id(date: &str) -> String { format!("unclassified:{date}") }

pub(crate) fn unclassified(date: &str) -> Result<CourseView, String> {
    let day = parse_date(date)?;
    let next = day.succ_opt().ok_or("日期越界")?;
    Ok(CourseView {
        id: unclassified_id(date), date: date.into(), slot_id: None, subject_id: None,
        subject_name: "未归类".into(), label: "未归类".into(), start_unix_ms: local_ms(day, 0)?,
        end_unix_ms: local_ms(next, 0)?, enabled: true, items: Vec::new(),
    })
}

/// 创建当天启用课次的不可变基础快照；非课程/禁用课程不伪造卡片。
pub(crate) fn snapshot(timetable: &Timetable, date: &str) -> Result<Vec<CourseView>, String> {
    let day = parse_date(date)?; let weekday = day.weekday().number_from_monday() as u8;
    let subjects: HashMap<_, _> = timetable.subjects.iter().map(|s| (s.id.as_str(), s.name.as_str())).collect();
    let mut result = Vec::new();
    for slot in &timetable.slots {
        let lesson = timetable.lessons.iter().find(|l| l.weekday == weekday && l.slot_id == slot.id && l.enabled);
        if !slot.is_class { continue; }
        if let Some(lesson) = lesson {
            let name = subjects.get(lesson.subject_id.as_str()).ok_or("课程科目不存在，不能创建快照")?;
            result.push(CourseView {
                id: format!("course:{date}:{}", slot.id), date: date.into(), slot_id: Some(slot.id.clone()),
                subject_id: Some(lesson.subject_id.clone()), subject_name: (*name).into(), label: slot.label.clone(),
                start_unix_ms: local_ms(day, time_minutes(&slot.start)?)?,
                end_unix_ms: local_ms(day, time_minutes(&slot.end)?)?, enabled: true, items: Vec::new(),
            });
        }
    }
    result.push(unclassified(date)?);
    Ok(result)
}

/// 首次检测时间在课程中归本课，在课间/课前归当天下一启用课；没有下一课返回 None。
/// 必须有对应日期的未归类日界快照，不能用上一日事件误配下一日。
pub fn classify_course<'a>(courses: &'a [CourseView], date: &str, detected: u64) -> Option<&'a str> {
    if !courses.iter().any(|c| c.date == date && c.slot_id.is_none() && c.start_unix_ms <= detected && detected < c.end_unix_ms) {
        return None;
    }
    courses.iter().filter(|c| c.date == date && c.enabled && c.slot_id.is_some() && detected < c.end_unix_ms)
        .min_by(|a, b| a.start_unix_ms.cmp(&b.start_unix_ms).then(a.id.cmp(&b.id))).map(|c| c.id.as_str())
}

pub fn import_timetable(text: &str, format: &str) -> Result<Timetable, String> {
    if text.len() > MAX_IMPORT_BYTES { return Err("课表导入不得超过 1 MiB".into()); }
    let timetable = match format {
        "json" => serde_json::from_str(text).map_err(|e| format!("课表 JSON 无效或含未知/缺失字段：{e}"))?,
        "csv" => import_csv(text)?,
        _ => return Err("课表格式只能是 csv 或 json".into()),
    };
    validate_timetable(timetable)
}

fn import_csv(text: &str) -> Result<Timetable, String> {
    let mut reader = csv::ReaderBuilder::new().has_headers(true).flexible(false)
        .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader.headers().map_err(|e| format!("CSV 表头无效：{e}"))?;
    if headers.iter().ne(CSV_HEADER) { return Err(format!("CSV 表头须精确为 {}", CSV_HEADER.join(","))); }
    let mut result = Timetable::default();
    for (index, row) in reader.records().enumerate() {
        if index >= 2048 { return Err("CSV 超过 2048 行上限".into()); }
        let row = row.map_err(|e| format!("CSV 第 {} 行无效：{e}", index + 2))?;
        if row.iter().any(|s| s.len() > 512) { return Err(format!("CSV 第 {} 行字段过长", index + 2)); }
        let boolean = |column: usize| -> Result<bool, String> {
            match &row[column] { "true" => Ok(true), "false" => Ok(false), _ => Err(format!("CSV 第 {} 行 {} 须为 true/false", index + 2, CSV_HEADER[column])) }
        };
        let empty_columns = |columns: &[usize]| -> Result<(), String> {
            if columns.iter().any(|i| !row[*i].is_empty()) { return Err(format!("CSV 第 {} 行包含此 kind 不适用的字段", index + 2)); }
            Ok(())
        };
        match &row[0] {
            "subject" => {
                empty_columns(&[3, 4, 5, 6, 7, 8, 9])?;
                result.subjects.push(Subject { id: row[1].to_owned(), name: row[2].to_owned() });
            }
            "slot" => {
                empty_columns(&[6, 7, 8, 9])?;
                result.slots.push(TimeSlot { id: row[1].to_owned(), label: row[2].to_owned(), start: row[3].to_owned(), end: row[4].to_owned(), is_class: boolean(5)? });
            }
            "lesson" => {
                empty_columns(&[1, 2, 3, 4, 5])?;
                result.lessons.push(WeekLesson { weekday: row[6].parse().map_err(|_| format!("CSV 第 {} 行星期无效", index + 2))?,
                    slot_id: row[7].to_owned(), subject_id: row[8].to_owned(), enabled: boolean(9)? });
            }
            _ => return Err(format!("CSV 第 {} 行 kind 只能是 subject/slot/lesson", index + 2)),
        }
    }
    Ok(result)
}

pub fn export_timetable(timetable: &Timetable, format: &str) -> Result<String, String> {
    let timetable = validate_timetable(timetable.clone())?;
    match format {
        "json" => serde_json::to_string_pretty(&timetable).map_err(|e| e.to_string()),
        "csv" => {
            let mut writer = csv::Writer::from_writer(Vec::new());
            writer.write_record(CSV_HEADER).map_err(|e| e.to_string())?;
            for s in &timetable.subjects { writer.write_record(["subject", &s.id, &s.name, "", "", "", "", "", "", ""]).map_err(|e| e.to_string())?; }
            for s in &timetable.slots { writer.write_record(["slot", &s.id, &s.label, &s.start, &s.end, if s.is_class {"true"} else {"false"}, "", "", "", ""]).map_err(|e| e.to_string())?; }
            for l in &timetable.lessons { writer.write_record(["lesson", "", "", "", "", "", &l.weekday.to_string(), &l.slot_id, &l.subject_id, if l.enabled {"true"} else {"false"}]).map_err(|e| e.to_string())?; }
            let bytes = writer.into_inner().map_err(|e| e.to_string())?;
            String::from_utf8(bytes).map_err(|e| e.to_string())
        }
        _ => Err("课表格式只能是 csv 或 json".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn timetable() -> Timetable {
        Timetable {
            subjects: vec![Subject {id:"math".into(),name:"数学".into()}],
            slots: vec![TimeSlot {id:"one".into(),label:"第一节".into(),start:"08:00".into(),end:"08:40".into(),is_class:true},
                TimeSlot {id:"break".into(),label:"课间".into(),start:"08:40".into(),end:"08:50".into(),is_class:false},
                TimeSlot {id:"two".into(),label:"第二节".into(),start:"08:50".into(),end:"09:30".into(),is_class:true}],
            lessons: vec![WeekLesson {weekday:1,slot_id:"one".into(),subject_id:"math".into(),enabled:true},
                WeekLesson {weekday:1,slot_id:"two".into(),subject_id:"math".into(),enabled:true}],
        }
    }
    #[test]
    fn dates_and_time_are_strict() {
        assert!(parse_date("2028-02-29").is_ok()); assert!(parse_date("2026-02-29").is_err());
        assert!(parse_date("2026-1-01").is_err()); assert!(time_minutes("24:00").is_err());
        assert!(time_minutes("8:00").is_err()); assert_eq!(time_minutes("08:40").unwrap(),520);
    }
    #[test]
    fn timetable_rejects_overlap_and_invalid_references() {
        assert!(validate_timetable(timetable()).is_ok());
        let mut t = timetable(); t.slots[1].start = "08:39".into(); assert!(validate_timetable(t).is_err());
        let mut t = timetable(); t.lessons[0].weekday = 0; assert!(validate_timetable(t).is_err());
        let mut t = timetable(); t.lessons[0].subject_id = "missing".into(); assert!(validate_timetable(t).is_err());
        let mut t = timetable(); t.lessons[0].slot_id = "break".into(); assert!(validate_timetable(t).is_err());
        let mut t = timetable(); t.lessons[0].enabled = false; t.lessons[0].subject_id.clear(); assert!(validate_timetable(t).is_ok());
    }
    #[test]
    fn same_subject_is_separate_courses_and_disabled_is_omitted() {
        let courses = snapshot(&timetable(), "2026-11-16").unwrap();
        assert_eq!(courses.len(),3); assert_ne!(courses[0].id,courses[1].id);
        let mut t = timetable(); t.lessons[1].enabled = false;
        assert_eq!(snapshot(&t,"2026-11-16").unwrap().len(),2);
    }
    #[test]
    fn first_time_break_and_end_of_day_classification() {
        let mut courses = snapshot(&timetable(),"2026-11-16").unwrap();
        let first = courses[0].clone(); let second = courses[1].clone();
        assert_eq!(classify_course(&courses,&first.date,first.start_unix_ms-1),Some(first.id.as_str()));
        assert_eq!(classify_course(&courses,&first.date,first.end_unix_ms),Some(second.id.as_str()));
        assert_eq!(classify_course(&courses,&first.date,second.end_unix_ms),None);
        assert_eq!(classify_course(&courses,"2026-11-17",first.start_unix_ms),None);
        courses[1].enabled=false;
        assert_eq!(classify_course(&courses,&first.date,first.end_unix_ms),None);
    }
    #[test]
    fn csv_and_json_round_trip_with_no_fake_rows() {
        let t = validate_timetable(timetable()).unwrap();
        for format in ["csv","json"] { assert_eq!(import_timetable(&export_timetable(&t,format).unwrap(),format).unwrap(),t); }
        assert!(import_timetable("kind,id\nsubject,one\n","csv").is_err());
        assert!(import_timetable("{\"subjects\":[],\"slots\":[],\"lessons\":[],\"fake\":1}","json").is_err());
        assert_eq!(import_timetable(&export_timetable(&Timetable::default(),"csv").unwrap(),"csv").unwrap(),Timetable::default());
    }
}
