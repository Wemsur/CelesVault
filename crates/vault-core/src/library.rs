use crate::api::LibraryQuery;
use regex::{Regex, RegexBuilder};

pub const MAX_QUERY_LIMIT: u32 = 200;
pub const MAX_QUERY_OFFSET: u64 = 1_000_000;
pub const MAX_PATTERN_BYTES: usize = 512;

/// Rust regex 无回溯/JS 引擎；编译和 DFA 内存均设小上限。
pub(crate) enum NameMatcher { Literal(String), Regex(Regex) }
impl NameMatcher {
    pub fn new(name: &str, regex: bool) -> Result<Self, String> {
        if name.len() > MAX_PATTERN_BYTES || name.contains('\0') { return Err("名称关键词/正则不得超过 512 字节或包含 NUL".into()); }
        if regex {
            RegexBuilder::new(name).case_insensitive(true).size_limit(256 * 1024).dfa_size_limit(512 * 1024).nest_limit(32)
                .build().map(Self::Regex).map_err(|e| format!("文件名正则无效或超出资源限制：{e}"))
        } else { Ok(Self::Literal(name.to_lowercase())) }
    }
    pub fn matches(&self, name: &str) -> bool {
        match self { Self::Literal(needle) => needle.is_empty() || name.to_lowercase().contains(needle), Self::Regex(regex) => regex.is_match(name) }
    }
}

pub fn validate_library_query(mut query: LibraryQuery) -> Result<LibraryQuery, String> {
    if !(1..=MAX_QUERY_LIMIT).contains(&query.limit) || query.offset > MAX_QUERY_OFFSET { return Err("库分页 limit 须为 1–200，offset 不得超过 1000000".into()); }
    if !["none", "date", "month", "subject", "course", "type"].contains(&query.group_by.as_str()) { return Err("未知库分组方式".into()); }
    if !["archived", "used", "name", "size"].contains(&query.sort_by.as_str()) { return Err("未知库排序方式".into()); }
    NameMatcher::new(&query.name,query.regex)?;
    if query.subjects.len() > 128 || query.extensions.len() > 128 { return Err("科目和格式筛选最多各 128 项".into()); }
    for subject in &query.subjects { crate::config::text(subject,"科目筛选",256,false)?; }
    if let Some(date)=&query.date_from { crate::timetable::parse_date(date)?; }
    if let Some(date)=&query.date_to { crate::timetable::parse_date(date)?; }
    if let (Some(from),Some(to))=(&query.date_from,&query.date_to) { if from>to { return Err("起始日期不能晚于结束日期".into()); } }
    for ext in &mut query.extensions { if ext.is_empty() { continue; } *ext=crate::config::extension(ext)?; }
    query.subjects.sort();query.subjects.dedup();query.extensions.sort();query.extensions.dedup();
    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regex_is_bounded_and_not_js_backtracking() {
        assert!(NameMatcher::new("(?=课程)",true).is_err());
        assert!(NameMatcher::new("(a)\\1",true).is_err());
        assert!(NameMatcher::new("[",true).is_err());
        assert!(NameMatcher::new(&"a".repeat(513),true).is_err());
        let regex=NameMatcher::new("(a+)+$",true).unwrap();
        assert!(!regex.matches(&format!("{}!","a".repeat(4096))));
        assert!(NameMatcher::new("数学.*\\.pdf$",true).unwrap().matches("数学复习.PDF"));
        assert!(NameMatcher::new("讲义",false).unwrap().matches("数学讲义.docx"));
    }
    #[test]
    fn query_dates_and_pagination_reject_invalid_values() {
        assert!(validate_library_query(LibraryQuery::default()).is_ok());
        let mut q=LibraryQuery::default();q.limit=201;assert!(validate_library_query(q).is_err());
        let mut q=LibraryQuery::default();q.offset=1_000_001;assert!(validate_library_query(q).is_err());
        let mut q=LibraryQuery::default();q.date_from=Some("2026-11-17".into());q.date_to=Some("2026-11-16".into());
        assert!(validate_library_query(q).is_err());
    }
}
