use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SMOKE_EXTENSIONS: &[&str] = &["docx", "pdf", "jpg"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub executable_path: Option<String>,
    pub parent_pid: Option<u32>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEvent {
    pub sequence: u64,
    pub event_unix_ms: u64,
    pub observed_unix_ms: u64,
    pub path: String,
    pub operation: String,
    pub event_id: u16,
    pub opcode: u8,
    pub version: u8,
    pub header_pid: u32,
    pub header_tid: u32,
    pub issuing_tid: Option<u32>,
    pub attributed_pid: Option<u32>,
    pub attribution_source: String,
    pub process: Option<ProcessInfo>,
    pub file_object: Option<u64>,
    pub file_key: Option<u64>,
    pub io_size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub unix_ms: u64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DriveInfo {
    pub root: String,
    pub drive_type: u32,
    pub bus_type: Option<i32>,
    pub selected_automatically: bool,
    pub nt_device: Option<String>,
    pub device_query_error: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "record_kind", content = "data", rename_all = "snake_case")]
pub enum LogRow {
    File(FileEvent),
    Foreground {
        unix_ms: u64,
        process: ProcessInfo,
    },
    Drives {
        unix_ms: u64,
        drives: Vec<DriveInfo>,
        effective_roots: Vec<String>,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Counters {
    pub callback_events: u64,
    pub file_events: u64,
    pub scope_matching_events: u64,
    pub queue_dropped: u64,
    pub schema_failures: u64,
    pub property_failures: u64,
    pub path_unresolved_events: u64,
    pub path_out_of_scope_events: u64,
    pub unknown_attribution_matching_events: u64,
    pub name_cache_resets: u64,
    pub callback_panics: u64,
    pub metadata_errors: BTreeMap<String, u64>,
}

impl Counters {
    pub fn error(&mut self, category: &str, code: u32, opcode: u8, version: u8) {
        let key = format!("{category}:win32={code}:opcode={opcode}:version={version}");
        if self.metadata_errors.contains_key(&key) || self.metadata_errors.len() < 32 {
            *self.metadata_errors.entry(key).or_default() += 1;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TraceHealth {
    pub events_lost: u32,
    pub realtime_buffers_lost: u32,
    pub stop_code: u32,
    pub close_code: u32,
    pub consumer_code: Option<u32>,
    pub consumer_joined: bool,
}
