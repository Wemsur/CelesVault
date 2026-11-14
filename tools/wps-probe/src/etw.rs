//! Real-time ETW controller. Only our unique system-logger session is stopped.
//! Classic file-name events call the name key "FileObject"; it is NOT an open handle's FileObject.
//! Known classic v2/v3 layouts follow the Windows kernel schema (also documented by PerfView).
use crate::{
    filter::{NameCache, Scope},
    model::{Counters, FileEvent, ProcessInfo, TraceHealth},
    windows::{self, ProcessTable, ThreadTable},
};
use std::{
    collections::HashMap,
    mem::{offset_of, size_of},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU32, Ordering},
        mpsc::{SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::Diagnostics::Etw::*};

#[derive(Default, Debug)]
struct ParsedFile {
    object: Option<u64>,
    key: Option<u64>,
    tid: Option<u32>,
    path: Option<String>,
    io_size: Option<u64>,
}

fn same_guid(a: windows_sys::core::GUID, b: windows_sys::core::GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

fn number(bytes: &[u8], at: usize, len: usize) -> Option<u64> {
    let slice = bytes.get(at..at.checked_add(len)?)?;
    match len {
        4 => Some(u32::from_le_bytes(slice.try_into().ok()?) as u64),
        8 => Some(u64::from_le_bytes(slice.try_into().ok()?)),
        _ => None,
    }
}
fn utf16(bytes: &[u8], at: usize) -> Option<String> {
    let bytes = bytes.get(at..)?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let end = units.iter().position(|c| *c == 0)?;
    Some(String::from_utf16_lossy(&units[..end]))
}
fn fast_file(op: u8, version: u8, pointer: usize, b: &[u8]) -> Option<ParsedFile> {
    if ![4, 8].contains(&pointer) {
        return None;
    }
    let delta = pointer - 4;
    let mut p = ParsedFile::default();
    match op {
        0 | 32 | 35 | 36 if version <= 3 => {
            p.key = number(b, 0, pointer);
            p.path = utf16(b, pointer);
            if p.key.is_none() || p.path.is_none() {
                return None;
            }
        }
        64 if version == 2 || version == 3 => {
            let (object, tid, name) = if version == 2 {
                (8 + 2 * delta, 4 + delta, 24 + 3 * delta)
            } else {
                (4 + delta, 8 + 2 * delta, 24 + 2 * delta)
            };
            p.object = number(b, object, pointer);
            p.tid = number(b, tid, if version == 2 { pointer } else { 4 }).map(|n| n as u32);
            p.path = utf16(b, name);
            if p.object.is_none() || p.tid.is_none() || p.path.is_none() {
                return None;
            }
        }
        67 | 68 if version == 2 || version == 3 => {
            let (object, key, tid, size) = if version == 2 {
                (16 + 2 * delta, 20 + 3 * delta, 12 + delta, 24 + 4 * delta)
            } else {
                (12 + delta, 16 + 2 * delta, 20 + 3 * delta, 24 + 3 * delta)
            };
            p.object = number(b, object, pointer);
            p.key = number(b, key, pointer);
            p.tid = number(b, tid, if version == 2 { pointer } else { 4 }).map(|n| n as u32);
            p.io_size = number(b, size, 4);
            if p.object.is_none() || p.key.is_none() || p.tid.is_none() || p.io_size.is_none() {
                return None;
            }
        }
        _ => return None,
    }
    Some(p)
}

#[derive(Clone)]
struct Property {
    name: String,
    wide_name: Vec<u16>,
    in_type: u16,
}
#[derive(Clone)]
struct Schema {
    properties: Vec<Property>,
}
type SchemaKey = (u8, u16, u8, u8, u16);

fn load_schema(record: &EVENT_RECORD) -> Result<Schema, u32> {
    let mut size = 0;
    let result =
        unsafe { TdhGetEventInformation(record, 0, ptr::null(), ptr::null_mut(), &mut size) };
    if result != ERROR_INSUFFICIENT_BUFFER || size == 0 || size > 1024 * 1024 {
        return Err(result);
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    let result = unsafe {
        TdhGetEventInformation(
            record,
            0,
            ptr::null(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result != 0 {
        return Err(result);
    }
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), size as usize) };
    if bytes.len() < offset_of!(TRACE_EVENT_INFO, EventPropertyInfoArray) {
        return Err(ERROR_INVALID_DATA);
    }
    let info = unsafe { &*buffer.as_ptr().cast::<TRACE_EVENT_INFO>() };
    let count = info.TopLevelPropertyCount as usize;
    let base = offset_of!(TRACE_EVENT_INFO, EventPropertyInfoArray);
    if count > 2048 || base + count * size_of::<EVENT_PROPERTY_INFO>() > bytes.len() {
        return Err(ERROR_INVALID_DATA);
    }
    let mut properties = Vec::new();
    for index in 0..count {
        let property = unsafe {
            &*bytes
                .as_ptr()
                .add(base + index * size_of::<EVENT_PROPERTY_INFO>())
                .cast::<EVENT_PROPERTY_INFO>()
        };
        let name = utf16(bytes, property.NameOffset as usize).ok_or(ERROR_INVALID_DATA)?;
        let in_type = unsafe { property.Anonymous1.nonStructType.InType };
        properties.push(Property {
            wide_name: windows::wide(&name),
            name,
            in_type,
        });
    }
    Ok(Schema { properties })
}

struct Fields<'a> {
    record: &'a EVENT_RECORD,
    schema: &'a Schema,
}
impl Fields<'_> {
    fn bytes(&self, names: &[&str]) -> Result<Option<(Vec<u8>, u16)>, u32> {
        let Some(property) = self
            .schema
            .properties
            .iter()
            .find(|p| names.iter().any(|n| p.name.eq_ignore_ascii_case(n)))
        else {
            return Ok(None);
        };
        let desc = PROPERTY_DATA_DESCRIPTOR {
            PropertyName: property.wide_name.as_ptr() as u64,
            ArrayIndex: u32::MAX,
            Reserved: 0,
        };
        let mut size = 0;
        let result =
            unsafe { TdhGetPropertySize(self.record, 0, ptr::null(), 1, &desc, &mut size) };
        if result != 0 {
            return Err(result);
        }
        if size > 65536 {
            return Err(ERROR_INVALID_DATA);
        }
        let mut buffer = vec![0u8; size as usize];
        let result = unsafe {
            TdhGetProperty(
                self.record,
                0,
                ptr::null(),
                1,
                &desc,
                size,
                buffer.as_mut_ptr(),
            )
        };
        if result != 0 {
            return Err(result);
        }
        Ok(Some((buffer, property.in_type)))
    }
    fn number(&self, names: &[&str]) -> Result<Option<u64>, u32> {
        Ok(self.bytes(names)?.and_then(|(b, _)| number(&b, 0, b.len())))
    }
    fn text(&self, names: &[&str]) -> Result<Option<String>, u32> {
        Ok(self.bytes(names)?.and_then(|(b, kind)| {
            if kind == TDH_INTYPE_ANSISTRING as u16 {
                Some(
                    String::from_utf8_lossy(
                        &b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())],
                    )
                    .into_owned(),
                )
            } else {
                utf16(&b, 0)
            }
        }))
    }
}

pub struct Collector {
    scope: Arc<RwLock<Scope>>,
    sender: SyncSender<FileEvent>,
    names: NameCache,
    processes: ProcessTable,
    threads: ThreadTable,
    schemas: HashMap<SchemaKey, Result<Schema, u32>>,
    pub counters: Counters,
}
impl Collector {
    pub fn new(scope: Arc<RwLock<Scope>>, sender: SyncSender<FileEvent>) -> Self {
        let (processes, threads) = windows::snapshot();
        Self {
            scope,
            sender,
            names: NameCache::default(),
            processes,
            threads,
            schemas: HashMap::new(),
            counters: Counters::default(),
        }
    }
    fn schema(&mut self, record: &EVENT_RECORD, category: u8) -> Option<Schema> {
        let d = &record.EventHeader.EventDescriptor;
        let key = (
            category,
            d.Id,
            d.Opcode,
            d.Version,
            record.EventHeader.Flags,
        );
        // Kernel schemas are a small, bounded set. Unknown schema failures are cached as well.
        if self.schemas.len() >= 256 && !self.schemas.contains_key(&key) {
            self.schemas.clear();
        }
        let result = self
            .schemas
            .entry(key)
            .or_insert_with(|| load_schema(record));
        match result {
            Ok(s) => Some(s.clone()),
            Err(code) => {
                self.counters.schema_failures += 1;
                self.counters.error("schema", *code, d.Opcode, d.Version);
                None
            }
        }
    }
    pub fn get_process(&mut self, pid: u32) -> Option<ProcessInfo> {
        if let Some(p) = self.processes.get_mut(&pid) {
            if p.executable_path.is_none() {
                p.executable_path = windows::process_path(pid);
            }
            if p.name.is_empty()
                && let Some(path) = &p.executable_path
            {
                p.name = path.rsplit('\\').next().unwrap_or(path).to_owned();
            }
            return Some(p.clone());
        }
        let p = windows::process_info(pid)?;
        if self.processes.len() < 16384 {
            self.processes.insert(pid, p.clone());
        }
        Some(p)
    }
    fn on_record(&mut self, record: &EVENT_RECORD) {
        self.counters.callback_events += 1;
        let header = &record.EventHeader;
        let op = header.EventDescriptor.Opcode;
        let version = header.EventDescriptor.Version;
        let provider = header.ProviderId;
        if same_guid(provider, ThreadGuid) {
            if [1, 2, 3, 4].contains(&op)
                && (2..=4).contains(&version)
                && !record.UserData.is_null()
                && record.UserDataLength >= 8
            {
                let data = unsafe {
                    std::slice::from_raw_parts(
                        record.UserData.cast::<u8>(),
                        record.UserDataLength as usize,
                    )
                };
                let pid = number(data, 0, 4).unwrap_or(0) as u32;
                let tid = number(data, 4, 4).unwrap_or(0) as u32;
                if op == 1 || op == 3 {
                    if self.threads.len() < 131072 {
                        self.threads.insert(tid, pid);
                    }
                } else {
                    self.threads.remove(&tid);
                }
            }
            return;
        }
        if same_guid(provider, ProcessGuid) {
            if [1, 2, 3, 4].contains(&op)
                && let Some(schema) = self.schema(record, 1)
            {
                let fields = Fields {
                    record,
                    schema: &schema,
                };
                if let Ok(Some(pid)) = fields.number(&["ProcessId", "ProcessID"]) {
                    let pid = pid as u32;
                    if op == 2 || op == 4 {
                        self.processes.remove(&pid);
                    } else {
                        // PID reuse invalidates the previous generation's name and executable.
                        let name = fields
                            .text(&["ImageFileName"])
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let parent = fields
                            .number(&["ParentId", "ParentProcessId"])
                            .ok()
                            .flatten()
                            .map(|n| n as u32);
                        if self.processes.len() < 16384 {
                            self.processes.insert(
                                pid,
                                ProcessInfo {
                                    pid,
                                    name,
                                    executable_path: None,
                                    parent_pid: parent,
                                    source: "etw_process_start_or_rundown".into(),
                                },
                            );
                        }
                    }
                }
            }
            return;
        }
        if !same_guid(provider, FileIoGuid) {
            return;
        }
        self.counters.file_events += 1;
        // Unknown operations stay counted, but are not mislabelled as file opens.
        if ![
            0, 32, 35, 36, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 77,
        ]
        .contains(&op)
        {
            return;
        }
        let pointer = if header.Flags & EVENT_HEADER_FLAG_32_BIT_HEADER as u16 != 0 {
            4
        } else {
            8
        };
        let data = if record.UserData.is_null() {
            &[][..]
        } else {
            unsafe {
                std::slice::from_raw_parts(
                    record.UserData.cast::<u8>(),
                    record.UserDataLength as usize,
                )
            }
        };
        let parsed = if let Some(p) = fast_file(op, version, pointer, data) {
            p
        } else {
            let Some(schema) = self.schema(record, 0) else {
                return;
            };
            let fields = Fields {
                record,
                schema: &schema,
            };
            let result = (|| -> Result<ParsedFile, u32> {
                Ok(ParsedFile {
                    object: fields.number(&["FileObject"])?,
                    key: fields.number(&["FileKey"])?,
                    tid: fields
                        .number(&["TTID", "IssuingThreadId"])?
                        .map(|n| n as u32),
                    path: fields.text(&["OpenPath", "FileName"])?,
                    io_size: fields.number(&["IoSize"])?,
                })
            })();
            match result {
                Ok(mut p) => {
                    if [0, 32, 35, 36].contains(&op) {
                        p.key = p.key.or(p.object.take());
                    }
                    p
                }
                Err(code) => {
                    self.counters.property_failures += 1;
                    self.counters.error("property", code, op, version);
                    return;
                }
            }
        };
        let scope = self.scope.read().unwrap_or_else(|p| p.into_inner());
        let resolved = if let Some(path) = &parsed.path {
            let path = scope.translate(path);
            let matched = scope.matches(&path).then_some(path);
            if self
                .names
                .insert(parsed.object, parsed.key, matched.clone())
            {
                self.counters.name_cache_resets += 1;
            }
            Some(matched)
        } else {
            self.names.lookup(parsed.object, parsed.key).cloned()
        };
        drop(scope);
        let path = match resolved {
            Some(Some(path)) => path,
            Some(None) => {
                self.counters.path_out_of_scope_events += 1;
                if op == 66 {
                    self.names.remove_object(parsed.object);
                }
                if op == 35 {
                    self.names.remove_key(parsed.key);
                }
                return;
            }
            None => {
                self.counters.path_unresolved_events += 1;
                return;
            }
        };
        // Validate again: scope or exclusion settings may have changed since the name event.
        if !self
            .scope
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .matches(&path)
        {
            return;
        }
        self.counters.scope_matching_events += 1;
        let (pid, source) = attribute(parsed.tid, header.ThreadId, header.ProcessId, &self.threads);
        if pid.is_none() {
            self.counters.unknown_attribution_matching_events += 1;
        }
        let process = pid.and_then(|pid| self.get_process(pid));
        let event = FileEvent {
            sequence: self.counters.scope_matching_events,
            event_unix_ms: windows::event_unix_ms(header.TimeStamp),
            observed_unix_ms: windows::unix_ms(),
            path,
            operation: operation(op).into(),
            event_id: header.EventDescriptor.Id,
            opcode: op,
            version,
            header_pid: header.ProcessId,
            header_tid: header.ThreadId,
            issuing_tid: parsed.tid,
            attributed_pid: pid,
            attribution_source: source.into(),
            process,
            file_object: parsed.object,
            file_key: parsed.key,
            io_size: parsed.io_size,
        };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.sender.try_send(event)
        {
            self.counters.queue_dropped += 1;
        }
        if op == 66 {
            self.names.remove_object(parsed.object);
        }
        if op == 35 {
            self.names.remove_key(parsed.key);
        }
    }
}

pub fn attribute(
    issuing: Option<u32>,
    header_tid: u32,
    header_pid: u32,
    threads: &ThreadTable,
) -> (Option<u32>, &'static str) {
    if let Some(tid) = issuing.filter(|n| *n != 0 && *n != u32::MAX) {
        return (
            threads.get(&tid).copied(),
            if threads.contains_key(&tid) {
                "issuing_thread_etw_or_startup_map"
            } else {
                "issuing_thread_unresolved_no_pid_guess"
            },
        );
    }
    if let Some(pid) = threads.get(&header_tid) {
        return (Some(*pid), "header_thread_etw_or_startup_map");
    }
    if header_pid != 0 && header_pid != u32::MAX {
        return (Some(header_pid), "event_header_pid_only");
    }
    (None, "unknown")
}
fn operation(op: u8) -> &'static str {
    match op {
        0 => "name_metadata",
        32 => "name_create_metadata",
        35 => "name_delete_metadata",
        36 => "name_rundown_metadata",
        64 => "open_create_handle",
        65 => "cleanup",
        66 => "close",
        67 => "read",
        68 => "write",
        69 => "set_info",
        70 => "delete",
        71 => "rename",
        72 => "directory_enum",
        73 => "flush",
        74 => "query_info",
        75 => "fs_control",
        77 => "directory_notify",
        _ => "unknown",
    }
}

struct CallbackContext {
    collector: Arc<Mutex<Collector>>,
    buffer_lost: AtomicU32,
}
unsafe extern "system" fn event_callback(raw: *mut EVENT_RECORD) {
    if raw.is_null() {
        return;
    }
    let record = unsafe { &*raw };
    if record.UserContext.is_null() {
        return;
    }
    let context = unsafe { &*record.UserContext.cast::<CallbackContext>() };
    let result = catch_unwind(AssertUnwindSafe(|| {
        context
            .collector
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .on_record(record)
    }));
    if result.is_err() {
        context
            .collector
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .counters
            .callback_panics += 1;
        windows::STOP_REQUESTED.store(true, Ordering::Relaxed);
    }
}
unsafe extern "system" fn buffer_callback(raw: *mut EVENT_TRACE_LOGFILEW) -> u32 {
    if !raw.is_null() {
        let log = unsafe { &*raw };
        if !log.Context.is_null() {
            let context = unsafe { &*log.Context.cast::<CallbackContext>() };
            context
                .buffer_lost
                .fetch_max(log.EventsLost, Ordering::Relaxed);
        }
    }
    1
}

fn properties(name: &[u16]) -> Vec<u64> {
    let size = size_of::<EVENT_TRACE_PROPERTIES>() + name.len() * 2;
    let mut bytes = vec![0u64; size.div_ceil(8)];
    unsafe {
        let p = &mut *bytes.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        p.Wnode.BufferSize = size as u32;
        // A custom system logger must not use the reserved NT Kernel Logger GUID.
        // GUID_NULL asks ETW to generate a unique identity for this session.
        p.Wnode.Guid = windows_sys::core::GUID::from_u128(0);
        p.Wnode.ClientContext = 1;
        p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        p.BufferSize = 64;
        p.MinimumBuffers = (thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            * 2)
        .clamp(8, 64) as u32;
        p.MaximumBuffers = 128;
        p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE;
        p.FlushTimer = 1;
        p.EnableFlags = EVENT_TRACE_FLAG_PROCESS
            | EVENT_TRACE_FLAG_THREAD
            | EVENT_TRACE_FLAG_FILE_IO
            | EVENT_TRACE_FLAG_FILE_IO_INIT;
        p.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            bytes
                .as_mut_ptr()
                .cast::<u8>()
                .add(size_of::<EVENT_TRACE_PROPERTIES>())
                .cast(),
            name.len(),
        );
    }
    bytes
}

pub struct TraceSession {
    pub name: String,
    wide_name: Vec<u16>,
    props: Vec<u64>,
    handle: CONTROLTRACE_HANDLE,
    trace: PROCESSTRACE_HANDLE,
    context: Option<Box<CallbackContext>>,
    logfile: Option<Box<EVENT_TRACE_LOGFILEW>>,
    consumer: Option<JoinHandle<u32>>,
    stopped: bool,
}
impl TraceSession {
    pub fn start(collector: Arc<Mutex<Collector>>) -> Result<Self, String> {
        windows::enable_trace_privilege()
            .map_err(|code| format!("无法启用当前进程的跟踪权限，Win32={code}"))?;
        let name = format!(
            "CelesVault-WPSProbe-{}-{}",
            std::process::id(),
            windows::unix_ms()
        );
        let mut wide_name = windows::wide(&name);
        let mut props = properties(&wide_name);
        let mut handle = CONTROLTRACE_HANDLE::default();
        let code =
            unsafe { StartTraceW(&mut handle, wide_name.as_ptr(), props.as_mut_ptr().cast()) };
        if code != 0 {
            return Err(format!(
                "ETW StartTraceW 失败：Win32={code}。未启动采集；不会停止其他跟踪会话。"
            ));
        }
        let mut context = Box::new(CallbackContext {
            collector,
            buffer_lost: AtomicU32::new(0),
        });
        let mut logfile = Box::<EVENT_TRACE_LOGFILEW>::default();
        logfile.LoggerName = wide_name.as_mut_ptr();
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(event_callback);
        logfile.BufferCallback = Some(buffer_callback);
        logfile.Context = (&mut *context as *mut CallbackContext).cast();
        let trace = unsafe { OpenTraceW(&mut *logfile) };
        if trace.Value == u64::MAX {
            let error = unsafe { GetLastError() };
            unsafe {
                ControlTraceW(
                    handle,
                    ptr::null(),
                    props.as_mut_ptr().cast(),
                    EVENT_TRACE_CONTROL_STOP,
                );
            };
            return Err(format!("ETW OpenTraceW 失败：Win32={error}"));
        }
        let value = trace.Value;
        let consumer = thread::spawn(move || unsafe {
            ProcessTrace(
                &PROCESSTRACE_HANDLE { Value: value },
                1,
                ptr::null(),
                ptr::null(),
            )
        });
        Ok(Self {
            name,
            wide_name,
            props,
            handle,
            trace,
            context: Some(context),
            logfile: Some(logfile),
            consumer: Some(consumer),
            stopped: false,
        })
    }
    pub fn consumer_finished(&self) -> bool {
        self.consumer.as_ref().is_some_and(|h| h.is_finished())
    }
    pub fn stop(&mut self) -> TraceHealth {
        if self.stopped {
            return TraceHealth::default();
        }
        self.stopped = true;
        let stop_code = unsafe {
            ControlTraceW(
                self.handle,
                ptr::null(),
                self.props.as_mut_ptr().cast(),
                EVENT_TRACE_CONTROL_STOP,
            )
        };
        let start = Instant::now();
        while !self.consumer_finished() && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(25));
        }
        let close_code = unsafe { CloseTrace(self.trace) };
        let start = Instant::now();
        while !self.consumer_finished() && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(25));
        }
        let joined = self.consumer_finished();
        let code = if joined {
            self.consumer.take().and_then(|h| h.join().ok())
        } else {
            None
        };
        let p = unsafe { &*self.props.as_ptr().cast::<EVENT_TRACE_PROPERTIES>() };
        let health = TraceHealth {
            events_lost: p.EventsLost.max(
                self.context
                    .as_ref()
                    .map(|c| c.buffer_lost.load(Ordering::Relaxed))
                    .unwrap_or(0),
            ),
            realtime_buffers_lost: p.RealTimeBuffersLost,
            stop_code,
            close_code,
            consumer_code: code,
            consumer_joined: joined,
        };
        if !joined {
            // A timed-out native consumer may still hold Context pointers. Do not free them.
            if let Some(context) = self.context.take() {
                Box::leak(context);
            }
            if let Some(logfile) = self.logfile.take() {
                Box::leak(logfile);
            }
            let _ = std::mem::take(&mut self.wide_name).leak();
        }
        health
    }
}
impl Drop for TraceSession {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.stop();
        }
    }
}

pub fn valid_session_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("CelesVault-WPSProbe-") else {
        return false;
    };
    let Some((pid, time)) = rest.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && !time.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && time.bytes().all(|b| b.is_ascii_digit())
        && pid.parse::<u32>().is_ok()
        && time.parse::<u64>().is_ok()
}
pub fn stop_named(name: &str) -> Result<(), String> {
    if !valid_session_name(name) {
        return Err("只允许停止日志中完整的 CelesVault-WPSProbe-数字-数字 会话名。".into());
    }
    let wide = windows::wide(name);
    let mut p = properties(&wide);
    let code = unsafe {
        ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            wide.as_ptr(),
            p.as_mut_ptr().cast(),
            EVENT_TRACE_CONTROL_STOP,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "停止指定诊断会话失败：Win32={code}（4201 通常表示会话已不存在）。"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_issuing_thread_does_not_guess_header_pid() {
        assert_eq!(
            attribute(Some(88), 1, 999, &HashMap::new()),
            (None, "issuing_thread_unresolved_no_pid_guess")
        );
    }
    #[test]
    fn issuing_thread_wins() {
        let m = HashMap::from([(8, 80), (9, 90)]);
        assert_eq!(attribute(Some(8), 9, 99, &m).0, Some(80));
    }
    #[test]
    fn invalid_pid_is_unknown() {
        assert_eq!(attribute(None, u32::MAX, u32::MAX, &HashMap::new()).0, None);
    }
    #[test]
    fn only_our_session_names() {
        assert!(valid_session_name("CelesVault-WPSProbe-123-1000"));
        assert!(!valid_session_name("NT Kernel Logger"));
        assert!(!valid_session_name("CelesVault-WPSProbe-123-1000-extra"));
    }
    #[test]
    fn independent_logger_does_not_use_reserved_guid() {
        let p = properties(&windows::wide("CelesVault-WPSProbe-1-1000"));
        let header = unsafe { &*p.as_ptr().cast::<EVENT_TRACE_PROPERTIES>() };
        assert!(same_guid(
            header.Wnode.Guid,
            windows_sys::core::GUID::from_u128(0)
        ));
        assert_ne!(header.LogFileMode & EVENT_TRACE_SYSTEM_LOGGER_MODE, 0);
    }
    #[test]
    fn truncated_payload_rejected() {
        assert!(fast_file(64, 3, 8, &[0; 16]).is_none());
    }
    #[test]
    fn classic_v3_create() {
        let mut b = vec![0u8; 32];
        b[8..16].copy_from_slice(&42u64.to_le_bytes());
        b[16..20].copy_from_slice(&123u32.to_le_bytes());
        b.extend(
            windows::wide("D:\\测试.docx")
                .iter()
                .flat_map(|n| n.to_le_bytes()),
        );
        let p = fast_file(64, 3, 8, &b).unwrap();
        assert_eq!(p.object, Some(42));
        assert_eq!(p.tid, Some(123));
        assert_eq!(p.path.as_deref(), Some("D:\\测试.docx"));
    }
    #[test]
    fn classic_v2_read() {
        let mut b = vec![0u8; 48];
        b[16..24].copy_from_slice(&8u64.to_le_bytes());
        b[24..32].copy_from_slice(&12u64.to_le_bytes());
        b[32..40].copy_from_slice(&15u64.to_le_bytes());
        b[40..44].copy_from_slice(&4096u32.to_le_bytes());
        let p = fast_file(67, 2, 8, &b).unwrap();
        assert_eq!(p.tid, Some(8));
        assert_eq!(p.object, Some(12));
        assert_eq!(p.key, Some(15));
        assert_eq!(p.io_size, Some(4096));
    }
    #[test]
    fn name_metadata_is_key_not_object() {
        let mut b = 42u64.to_le_bytes().to_vec();
        b.extend(
            windows::wide("D:\\a.jpg")
                .iter()
                .flat_map(|n| n.to_le_bytes()),
        );
        let p = fast_file(36, 2, 8, &b).unwrap();
        assert_eq!(p.key, Some(42));
        assert_eq!(p.object, None);
    }
}
