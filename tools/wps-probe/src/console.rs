use std::{collections::HashSet, time::Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum PreviewDecision {
    Print,
    Duplicate,
    LimitReached,
    Suppressed,
    Ignored,
}

// Console output is a bounded, per-phase preview; it never changes the event log.
pub struct ConsolePreview {
    seen: HashSet<(String, Option<u32>, String)>,
    limit: usize,
    suppressed: u64,
    phase_started: Instant,
    limit_announced: bool,
}
impl ConsolePreview {
    pub fn new(limit: usize, now: Instant) -> Self {
        Self {
            seen: HashSet::new(),
            limit,
            suppressed: 0,
            phase_started: now,
            limit_announced: false,
        }
    }
    pub fn reset_phase(&mut self, now: Instant) {
        self.seen.clear();
        self.phase_started = now;
        self.limit_announced = false;
    }
    pub fn displayed_count(&self) -> usize {
        self.seen.len()
    }
    pub fn suppressed_total(&self) -> u64 {
        self.suppressed
    }
    pub fn consider(
        &mut self,
        path: &str,
        pid: Option<u32>,
        operation: &str,
        now: Instant,
    ) -> PreviewDecision {
        if !["open_create_handle", "read"].contains(&operation) {
            return PreviewDecision::Ignored;
        }
        if now.saturating_duration_since(self.phase_started) >= std::time::Duration::from_secs(30) {
            self.reset_phase(now);
        }
        let key = (
            crate::filter::normalize(path).to_lowercase(),
            pid,
            operation.to_owned(),
        );
        if self.seen.contains(&key) {
            self.suppressed += 1;
            return PreviewDecision::Duplicate;
        }
        if self.seen.len() >= self.limit {
            self.suppressed += 1;
            if self.limit_announced {
                return PreviewDecision::Suppressed;
            }
            self.limit_announced = true;
            return PreviewDecision::LimitReached;
        }
        self.seen.insert(key);
        PreviewDecision::Print
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn a_new_marker_can_show_the_same_file_again() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(100, now);
        assert_eq!(
            p.consider("D:/a.docx", Some(1), "open_create_handle", now),
            PreviewDecision::Print
        );
        assert_eq!(
            p.consider("D:/a.docx", Some(1), "open_create_handle", now),
            PreviewDecision::Duplicate
        );
        p.reset_phase(now + Duration::from_secs(1));
        assert_eq!(
            p.consider(
                "D:/a.docx",
                Some(1),
                "open_create_handle",
                now + Duration::from_secs(1)
            ),
            PreviewDecision::Print
        );
    }
    #[test]
    fn full_phase_does_not_hide_an_image_in_the_next_phase() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(1, now);
        assert_eq!(
            p.consider("D:/a.docx", Some(1), "read", now),
            PreviewDecision::Print
        );
        let _ = p.consider("D:/page-05.jpg", Some(2), "read", now);
        p.reset_phase(now + Duration::from_secs(1));
        assert_eq!(
            p.consider(
                "D:/page-05.jpg",
                Some(2),
                "read",
                now + Duration::from_secs(1)
            ),
            PreviewDecision::Print
        );
    }
    #[test]
    fn unmarked_capture_reopens_the_preview_after_thirty_seconds() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(1, now);
        assert_eq!(
            p.consider("D:/a.jpg", Some(1), "read", now),
            PreviewDecision::Print
        );
        assert_eq!(
            p.consider("D:/a.jpg", Some(1), "read", now + Duration::from_secs(30)),
            PreviewDecision::Print
        );
    }
    #[test]
    fn case_and_slashes_do_not_consume_duplicate_slots() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(100, now);
        assert_eq!(
            p.consider("D:/Samples/PAGE.JPG", Some(1), "read", now),
            PreviewDecision::Print
        );
        assert_eq!(
            p.consider(r"d:\samples\page.jpg", Some(1), "read", now),
            PreviewDecision::Duplicate
        );
    }
    #[test]
    fn open_and_read_are_separately_visible() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(100, now);
        assert_eq!(
            p.consider("D:/a.pdf", Some(1), "open_create_handle", now),
            PreviewDecision::Print
        );
        assert_eq!(
            p.consider("D:/a.pdf", Some(1), "read", now),
            PreviewDecision::Print
        );
    }
    #[test]
    fn cap_is_announced_once_instead_of_silently_stopping() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(1, now);
        let _ = p.consider("D:/a.docx", Some(1), "read", now);
        assert_eq!(
            p.consider("D:/b.pdf", Some(2), "read", now),
            PreviewDecision::LimitReached
        );
        assert_eq!(
            p.consider("D:/c.jpg", Some(3), "read", now),
            PreviewDecision::Suppressed
        );
        assert_eq!(p.displayed_count(), 1);
        assert_eq!(p.suppressed_total(), 2);
    }
    #[test]
    fn metadata_is_not_printed_as_an_open() {
        let now = Instant::now();
        let mut p = ConsolePreview::new(100, now);
        assert_eq!(
            p.consider("D:/a.jpg", Some(1), "name_rundown_metadata", now),
            PreviewDecision::Ignored
        );
    }
}
