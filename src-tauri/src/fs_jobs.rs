// SPDX-License-Identifier: GPL-3.0-only
//! 真实 Windows 目录、冲突审批、归档导出和显式文件管理。
//! 归档 ExportSource 只能由宿主在 ArchiveLease 存活期间构造；启动返回前接手全批
//! 只读来源句柄。单复制 worker、有界计划/历史/事件，不执行用户文件或任意命令。
//! 参考 Sigma 的目录/复制任务分层，不引入其网络、移动、终端或无界事件模块。

use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread::JoinHandle};

const MAX_FINISHED_JOBS: usize = 32;
const UNSUPPORTED: &str = "真实目录、文件操作与导出仅支持 Windows 本地盘，未回退到模拟模式";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub root: String,
    pub name: String,
    pub identity: String,
    pub device_id: Option<String>,
    pub volume_id: String,
    pub identity_reliable: bool,
    pub is_usb: bool,
    pub can_eject: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub identity: String,
    pub extension: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    pub path: String,
    pub identity: String,
    pub volume_id: String,
    pub writable: bool,
    pub entries: Vec<DirectoryEntry>,
}

/// Rust-only 来源，不实现 Deserialize；前端不得任意指定读取/导出来源路径。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSource { pub id: String, pub display_name: String, pub path: PathBuf }

fn default_conflict() -> String { "keepBoth".into() }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CopyRequest {
    pub target_path: String,
    pub target_identity: String,
    #[serde(default = "default_conflict")]
    pub conflict: String,
}
impl Default for CopyRequest {
    fn default() -> Self { Self { target_path: String::new(), target_identity: String::new(), conflict: default_conflict() } }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    pub source_id: String,
    pub source_name: String,
    pub target_name: String,
    pub exists: bool,
    pub existing_size: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyPlan { pub id: String, pub items: Vec<ConflictItem> }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictResolution { pub source_id: String, pub policy: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyJob {
    pub id: String,
    pub state: String,
    /// 去重后的来源文件数，不是字节数；全部 skip 仍是一个真实结束任务。
    pub total: u64,
    /// 已尝试的来源数，包含当前失败项；不代表全部已成功。
    pub processed: u64,
    /// 已完成内容复核、同步及目标校验的副本数；仅rename成功不计入。
    pub copied: u64,
    pub skipped: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecovery {
    pub id: String,
    pub kind: String,
    pub original_name: String,
    pub held_name: String,
    pub held_path: String,
    pub can_restore: bool,
    pub error: Option<String>,
}

/// 宿主交给系统打开/定位请求时持有；不把路径校验与句柄释放之间的窗口藏起来。
pub struct BrowserEntryLease {
    pub path: PathBuf,
    #[cfg(windows)]
    _entry: native::BrowserRead,
}

#[derive(Default)]
struct Registry { jobs: VecDeque<CopyJob> }
struct Worker { id: String, cancel: Arc<AtomicBool>, handle: JoinHandle<()>, volumes: Vec<String> }
impl Worker {
    fn request_cancel(&self) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        #[cfg(windows)]
        { cancel_thread(&self.handle)?; }
        Ok(())
    }
}

#[cfg(windows)]
fn cancel_thread(handle: &JoinHandle<()>) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::ERROR_NOT_FOUND, System::IO::CancelSynchronousIo};
    if handle.is_finished() { return Ok(()); }
    // SAFETY: JoinHandle 在整个调用期间存活且未 join，不缓存或复用裸线程句柄。
    if unsafe { CancelSynchronousIo(handle.as_raw_handle().cast()) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
            return Err(crate::path_guard::io_error("取消已请求，但设备同步 I/O 未能中断；不会强杀线程", error));
        }
    }
    Ok(())
}

pub struct FsJobs {
    registry: Arc<Mutex<Registry>>,
    lifecycle: Mutex<()>,
    worker: Mutex<Option<Worker>>,
    stopped: AtomicBool,
    #[cfg(windows)]
    plans: Mutex<VecDeque<native::Plan>>,
    #[cfg(windows)]
    watcher: Mutex<Option<native::WatchWorker>>,
    #[cfg(windows)]
    watch_error: Arc<Mutex<Option<String>>>,
}

impl FsJobs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            registry: Arc::new(Mutex::new(Registry::default())), lifecycle: Mutex::new(()),
            worker: Mutex::new(None), stopped: AtomicBool::new(false),
            #[cfg(windows)] plans: Mutex::new(VecDeque::new()),
            #[cfg(windows)] watcher: Mutex::new(None),
            #[cfg(windows)] watch_error: Arc::new(Mutex::new(None)),
        })
    }
    fn ensure_running(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) { Err("文件模块正在退出，不能开始新操作".into()) } else { Ok(()) }
    }

    pub fn list_drives(&self) -> Result<Vec<DriveInfo>, String> {
        self.ensure_running()?;
        crate::devices::list()
    }
    pub fn browse(&self, path: String, data_root: Option<String>) -> Result<DirectoryView, String> {
        self.browse_options(path, data_root, false)
    }
    pub fn browse_options(&self, path: String, data_root: Option<String>, show_hidden: bool) -> Result<DirectoryView, String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::browse(path, data_root, show_hidden) }
        #[cfg(not(windows))] { let _ = (path, data_root, show_hidden); Err(UNSUPPORTED.into()) }
    }

    pub fn preview_copy(&self, sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<CopyPlan, String> {
        self.ensure_running()?;
        #[cfg(windows)] {
            let _lifecycle = self.lifecycle.lock().map_err(|_| "复制审批生命周期锁失败")?;
            self.ensure_running()?;
            let plan = native::preview(sources, request, data_root)?;
            let view = plan.view.clone();
            let mut plans = self.plans.lock().map_err(|_| "复制审批计划锁失败")?;
            native::prune_plans(&mut plans);
            while plans.len() >= native::MAX_PLANS { plans.pop_front(); }
            plans.push_back(plan);
            Ok(view)
        }
        #[cfg(not(windows))] { let _ = (sources, request, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn plan_ids(&self, plan_id: String) -> Result<Vec<String>, String> {
        self.ensure_running()?;
        #[cfg(windows)] {
            let mut plans = self.plans.lock().map_err(|_| "读取复制审批计划失败")?;
            native::prune_plans(&mut plans);
            let plan = plans.iter().find(|plan| plan.view.id == plan_id).ok_or("复制审批已过期或被淘汰，请重新预览")?;
            Ok(plan.sources.iter().map(|source| source.id.clone()).collect())
        }
        #[cfg(not(windows))] { let _ = plan_id; Err(UNSUPPORTED.into()) }
    }
    pub fn start_copy(&self, sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<CopyJob, String> {
        #[cfg(windows)] { self.start_native(move || native::prepare(sources, request, data_root)) }
        #[cfg(not(windows))] { let _ = (sources, request, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn start_plan(&self, sources: Vec<ExportSource>, plan_id: String, resolutions: Vec<ConflictResolution>, data_root: Option<String>) -> Result<CopyJob, String> {
        #[cfg(windows)] {
            self.start_native(move || {
                let mut plans = self.plans.lock().map_err(|_| "读取复制审批计划失败")?;
                native::prune_plans(&mut plans);
                let index = plans.iter().position(|plan| plan.view.id == plan_id).ok_or("复制审批已过期、已使用或被淘汰，请重新预览")?;
                // 批准一次性消费，即便后续核验失败也必须重新显示冲突，不复用过期批准。
                let plan = plans.remove(index).ok_or("复制审批计划缺失")?;
                drop(plans);
                native::prepare_plan(sources, plan, resolutions, data_root)
            })
        }
        #[cfg(not(windows))] { let _ = (sources, plan_id, resolutions, data_root); Err(UNSUPPORTED.into()) }
    }

    pub fn jobs(&self) -> Result<Vec<CopyJob>, String> {
        #[cfg(windows)] { Ok(self.registry.lock().map_err(|_| "读取复制任务列表失败")?.jobs.iter().rev().cloned().collect()) }
        #[cfg(not(windows))] { Err(UNSUPPORTED.into()) }
    }
    pub fn cancel(&self, id: String) -> Result<(), String> {
        #[cfg(windows)] {
            if id.is_empty() || id.len() > 128 { return Err("复制任务 ID 无效".into()); }
            let slot = self.worker.lock().map_err(|_| "访问复制任务线程失败")?;
            if let Some(worker) = slot.as_ref().filter(|worker| worker.id == id && !worker.handle.is_finished()) { return worker.request_cancel(); }
            let registry = self.registry.lock().map_err(|_| "读取复制任务列表失败")?;
            if registry.jobs.iter().any(|job| job.id == id) { Err("复制任务已结束，无需取消；已提交的导出不会撤销".into()) } else { Err("未找到复制任务".into()) }
        }
        #[cfg(not(windows))] { let _ = id; Err(UNSUPPORTED.into()) }
    }
    pub fn busy_volumes(&self) -> Result<Vec<String>, String> {
        #[cfg(windows)] {
            let mut volumes = Vec::new();
            let worker = self.worker.lock().map_err(|_| "读取复制活动卷失败")?;
            if let Some(worker) = worker.as_ref().filter(|worker| !worker.handle.is_finished()) { volumes.extend(worker.volumes.clone()); }
            drop(worker);
            let watcher = self.watcher.lock().map_err(|_| "读取目录监听活动卷失败")?;
            if let Some(watcher) = watcher.as_ref().filter(|watcher| !watcher.handle.is_finished()) { volumes.extend(watcher.volumes.clone()); }
            volumes.sort(); volumes.dedup(); Ok(volumes)
        }
        #[cfg(not(windows))] { Err(UNSUPPORTED.into()) }
    }

    pub fn validated_entry(&self, path: String, identity: String, data_root: Option<String>) -> Result<PathBuf, String> {
        Ok(self.lease_browser_entry(path, identity, data_root)?.path.clone())
    }
    pub fn lease_browser_entry(&self, path: String, identity: String, data_root: Option<String>) -> Result<BrowserEntryLease, String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::lease_browser_entry(path, identity, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, data_root); Err(UNSUPPORTED.into()) }
    }

    pub fn create_folder(&self, path: String, identity: String, name: String, data_root: Option<String>) -> Result<(), String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::create_folder(path, identity, name, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, name, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn rename_entry(&self, path: String, identity: String, name: String, data_root: Option<String>) -> Result<(), String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::rename_entry(path, identity, name, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, name, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn recycle_entry(&self, path: String, identity: String, data_root: Option<String>) -> Result<(), String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::recycle_entry(path, identity, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn file_recoveries(&self, path: String, identity: String, data_root: Option<String>) -> Result<Vec<FileRecovery>, String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::file_recoveries(path, identity, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, data_root); Err(UNSUPPORTED.into()) }
    }
    pub fn restore_file_recovery(&self, path: String, identity: String, recovery_id: String, data_root: Option<String>) -> Result<(), String> {
        self.ensure_running()?;
        #[cfg(windows)] { native::restore_file_recovery(path, identity, recovery_id, data_root) }
        #[cfg(not(windows))] { let _ = (path, identity, recovery_id, data_root); Err(UNSUPPORTED.into()) }
    }

    pub fn watch_directory(&self, path: String, identity: String, data_root: Option<String>, callback: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        self.ensure_running()?;
        #[cfg(windows)] {
            let _lifecycle = self.lifecycle.lock().map_err(|_| "目录监听生命周期锁失败")?;
            self.ensure_running()?;
            self.stop_watch()?;
            *self.watch_error.lock().map_err(|_| "目录监听状态锁失败")? = None;
            let worker = native::start_watch(path, identity, data_root, callback, self.watch_error.clone())?;
            let mut slot = self.watcher.lock().map_err(|_| "目录监听线程锁失败")?;
            *slot = Some(worker);
            if let Err(start_error) = slot.as_mut().ok_or("目录监听线程缺失")?.wait_ready() {
                match slot.as_mut().ok_or("目录监听线程缺失")?.stop() {
                    Ok(()) => {
                        let worker = slot.take().ok_or("目录监听线程缺失")?;
                        drop(slot);
                        worker.handle.join().map_err(|_| "目录监听启动线程异常结束")?;
                        return Err(start_error);
                    }
                    Err(stop_error) => return Err(format!("{start_error}；{stop_error}")),
                }
            }
            Ok(())
        }
        #[cfg(not(windows))] { let _ = (path, identity, data_root, callback); Err(UNSUPPORTED.into()) }
    }
    pub fn unwatch_directory(&self) -> Result<(), String> {
        #[cfg(windows)] {
            let _lifecycle = self.lifecycle.lock().map_err(|_| "目录监听生命周期锁失败")?;
            self.stop_watch()
        }
        #[cfg(not(windows))] { Err(UNSUPPORTED.into()) }
    }
    pub fn watch_error(&self) -> Result<Option<String>, String> {
        #[cfg(windows)] { Ok(self.watch_error.lock().map_err(|_| "读取目录监听状态失败")?.clone()) }
        #[cfg(not(windows))] { Err(UNSUPPORTED.into()) }
    }
    #[cfg(windows)]
    fn stop_watch(&self) -> Result<(), String> {
        let mut slot = self.watcher.lock().map_err(|_| "访问目录监听线程失败")?;
        if let Some(watcher) = slot.as_mut() { watcher.stop()?; }
        if let Some(watcher) = slot.take() { watcher.handle.join().map_err(|_| "目录监听线程异常结束")?; }
        Ok(())
    }

    /// 复制必须 cancel + join；不能设置 cancel 后立即 app.exit。设备驱动若不响应
    /// CancelSynchronousIo，复制 join 仍须等待，不能强杀。监听退出单独有 2 秒上限。
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|poison| poison.into_inner());
        #[cfg(windows)] {
            self.plans.lock().unwrap_or_else(|poison| poison.into_inner()).clear();
            if let Err(error) = self.stop_watch() { *self.watch_error.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(error); }
        }
        let worker = self.worker.lock().unwrap_or_else(|poison| poison.into_inner()).take();
        if let Some(worker) = worker {
            if let Err(error) = worker.request_cancel() {
                let mut registry = self.registry.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == worker.id && job.state == "running") { job.error = Some(error); }
            }
            if worker.handle.join().is_err() {
                let mut registry = self.registry.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == worker.id) { job.state = "failed".into(); job.error = Some("复制线程异常结束，未确认文件不记为成功".into()); }
                trim_finished(&mut registry);
            }
        }
    }

    #[cfg(windows)]
    fn start_native(&self, prepare: impl FnOnce() -> Result<native::Prepared, String>) -> Result<CopyJob, String> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| "复制生命周期锁失败")?;
        self.ensure_running()?;
        let mut slot = self.worker.lock().map_err(|_| "访问复制线程失败")?;
        if slot.as_ref().is_some_and(|worker| !worker.handle.is_finished()) { return Err("已有复制任务正在运行，请等待或取消后再复制".into()); }
        if let Some(previous) = slot.take() {
            if previous.handle.join().is_err() {
                let mut registry = self.registry.lock().map_err(|_| "更新任务记录失败")?;
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == previous.id) { job.state = "failed".into(); job.error = Some("上一复制线程异常结束".into()); }
            }
        }
        let prepared = prepare()?;
        let id = native::unique_id();
        let snapshot = CopyJob { id: id.clone(), state: "running".into(), total: prepared.source_count(), processed: 0, copied: 0, skipped: 0, error: None };
        let volumes = prepared.volumes.clone();
        {
            let mut registry = self.registry.lock().map_err(|_| "创建复制记录失败")?;
            trim_finished(&mut registry); registry.jobs.push_back(snapshot.clone());
        }
        let cancel = Arc::new(AtomicBool::new(false)); let worker_cancel = cancel.clone();
        let registry = self.registry.clone(); let current = snapshot.clone();
        let handle = match std::thread::Builder::new().name("celesvault-export".into()).spawn(move || native::run(prepared, current, worker_cancel, registry)) {
            Ok(handle) => handle,
            Err(error) => {
                let mut failed = snapshot.clone(); failed.state = "failed".into(); failed.error = Some(crate::path_guard::io_error("启动复制线程失败", error));
                finish_snapshot(&self.registry, &failed); return Err(failed.error.unwrap_or_else(|| "启动复制线程失败".into()));
            }
        };
        *slot = Some(Worker { id, cancel, handle, volumes });
        // 全批只读来源句柄已经归工作任务所有，此时宿主才可 drop ArchiveLease。
        Ok(snapshot)
    }
}
impl Drop for FsJobs { fn drop(&mut self) { self.shutdown(); } }
fn trim_finished(registry: &mut Registry) {
    while registry.jobs.iter().filter(|job| job.state != "running").count() > MAX_FINISHED_JOBS {
        if let Some(index) = registry.jobs.iter().position(|job| job.state != "running") { registry.jobs.remove(index); } else { break; }
    }
}
#[cfg(windows)]
fn finish_snapshot(registry: &Arc<Mutex<Registry>>, current: &CopyJob) {
    let mut registry = registry.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(stored) = registry.jobs.iter_mut().find(|job| job.id == current.id) { *stored = current.clone(); }
    trim_finished(&mut registry);
}
#[cfg(windows)]
pub(crate) fn raw_drives() -> Result<Vec<DriveInfo>, String> { native::list_drives() }

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{devices, path_guard::{
        ArchiveFile, DirectoryGuard, EntryGuard, FileKey, FileStamp, OwnedTemp, ProtectedRoot,
        conflict_name, file_hash, file_stamp, final_path, io_error, local_path, rename_no_replace,
        safe_name, same_path, user_name, wide,
    }};
    use sha2::{Digest, Sha256};
    use std::{
        collections::{HashMap, HashSet}, fs::{self, File, OpenOptions},
        io::{self, Read, Write}, mem::size_of,
        os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::{AsRawHandle, FromRawHandle, OwnedHandle}},
        panic::{AssertUnwindSafe, catch_unwind}, ptr,
        sync::{atomic::AtomicU64, mpsc::{self, Receiver}},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR, ERROR_OPERATION_ABORTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM, FILE_ATTRIBUTE_TEMPORARY,
            FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo, GetDriveTypeW, GetLogicalDrives,
            FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, ReadDirectoryChangesW,
            SetFileInformationByHandle,
        },
        System::{IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED}, Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject}, WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE}},
    };

    const MAX_DIRECTORY_ENTRIES: usize = 10_000;
    const MAX_COPY_SOURCES: usize = 1000;
    const MAX_NAME_ATTEMPTS: usize = 10_000;
    const MAX_TEMP_ATTEMPTS: usize = 32;
    const CHUNK_SIZE: usize = 64 * 1024;
    const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(200);
    pub(super) const MAX_PLANS: usize = 8;
    const PLAN_TTL: Duration = Duration::from_secs(300);
    const MAX_INTENT_BYTES: u64 = 32 * 1024;
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    pub(super) fn unique_id() -> String {
        let ticks = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!("export-{:x}-{ticks:x}-{sequence:x}", std::process::id())
    }
    fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }
    fn protected_root(data_root: Option<String>) -> Result<ProtectedRoot, String> {
        ProtectedRoot::open(data_root.as_deref().ok_or("尚未配置持久目录，不能执行文件操作")?)
    }
    fn reject_system(volume: &str) -> Result<(), String> {
        if devices::normalize_volume_identifier(volume)? == devices::system_volume()? { return Err("第一版不修改或回收 Windows 系统卷中的浏览文件".into()); }
        Ok(())
    }
    fn selected_target(path: &str, identity: &str, protected: &ProtectedRoot, write: bool) -> Result<DirectoryGuard, String> {
        if identity.is_empty() || identity.len() > 32_768 { return Err("缺少有效目标身份，请先浏览并选择目录".into()); }
        let guard = if write { DirectoryGuard::for_copy(path)? } else { DirectoryGuard::open(path)? };
        if guard.identity != identity { return Err("目标目录或卷身份已变化，请刷新并重新选择；未开始操作".into()); }
        protected.reject_target(&guard)?;
        Ok(guard)
    }
    pub(super) fn list_drives() -> Result<Vec<DriveInfo>, String> {
        let mask = unsafe { GetLogicalDrives() };
        if mask == 0 { return Err(io_error("枚举本地驱动器失败", io::Error::last_os_error())); }
        let mut drives = Vec::new();
        for index in 0..26 {
            if mask & (1u32 << index) == 0 { continue; }
            let root = format!("{}:\\", char::from(b'A' + index as u8));
            let kind = unsafe { GetDriveTypeW(wide(&root).as_ptr()) };
            if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE { continue; }
            match fs::symlink_metadata(&root) {
                Err(error) if matches!(error.raw_os_error(), Some(2) | Some(3) | Some(21)) => continue,
                Err(error) => return Err(io_error(&format!("读取驱动器 {root} 失败"), error)), Ok(_) => (),
            }
            let guard = DirectoryGuard::open(&root)?;
            let label = if guard.label().is_empty() { if kind == DRIVE_REMOVABLE { "可移动磁盘" } else { "本地磁盘" } } else { guard.label() };
            drives.push(DriveInfo { root: root.clone(), name: format!("{label} ({})", root.trim_end_matches('\\')), identity: guard.identity.clone(),
                volume_id: guard.volume_id()?, device_id: None, identity_reliable: false, is_usb: false, can_eject: false });
        }
        Ok(drives)
    }
    fn internal_hidden(name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        name.starts_with(".celesvault-") || name.starts_with("~$") || matches!(name.as_str(), "$recycle.bin" | "system volume information")
    }
    pub(super) fn browse(path: String, data_root: Option<String>, show_hidden: bool) -> Result<DirectoryView, String> {
        let guard = DirectoryGuard::open(&path)?;
        let protected = data_root.as_deref().map(ProtectedRoot::open).transpose()?;
        if let Some(protected) = &protected { protected.reject_target(&guard)?; }
        let read = fs::read_dir(&guard.final_path).map_err(|error| io_error("读取目录内容失败", error))?;
        let mut entries = Vec::new();
        for (index, entry) in read.enumerate() {
            if index >= MAX_DIRECTORY_ENTRIES { return Err("目录超过 10000 项，不能返回不完整列表；请进入更小的子目录".into()); }
            let entry = entry.map_err(|error| io_error("枚举目录项失败，请刷新后重试", error))?;
            let name = entry.file_name().into_string().map_err(|_| "目录项名称编码不可安全表示")?;
            if internal_hidden(&name) || (!show_hidden && name.starts_with('.')) { continue; }
            let real_path = entry.path();
            if protected.as_ref().is_some_and(|protected| protected.hides_entry(&real_path)) { continue; }
            let metadata = fs::symlink_metadata(&real_path).map_err(|error| io_error("读取目录项元数据失败，请刷新", error))?;
            let attributes = metadata.file_attributes();
            if attributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_TEMPORARY) != 0
                || (!show_hidden && attributes & FILE_ATTRIBUTE_HIDDEN != 0) || (!metadata.is_file() && !metadata.is_dir()) { continue; }
            let display_path = guard.path.join(&name);
            local_path(display_path.to_str().ok_or("目录项路径编码无效")?)?;
            let file = guard.open_entry(&name, false).map_err(|error| io_error("读取目录项句柄身份失败，请刷新", error))?;
            let stamp = file_stamp(&file)?; let final_name = final_path(&file)?;
            let identity = guard.entry_identity(&stamp, &final_name)?;
            if file_stamp(&file)? != stamp { return Err("目录项在读取期间变化，请刷新后重试".into()); }
            let is_dir = stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let extension = if is_dir { String::new() } else { std::path::Path::new(&name).extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase() };
            let modified_unix_ms = (stamp.modified / 10_000).saturating_sub(11_644_473_600_000);
            entries.push(DirectoryEntry { name, path: display_path.to_string_lossy().into_owned(), identity, extension, is_dir, size: if is_dir { 0 } else { stamp.size }, modified_unix_ms });
        }
        guard.validate()?;
        if let Some(protected) = &protected { protected.reject_target(&guard)?; }
        entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));
        let writable = guard.writable()?;
        Ok(DirectoryView { path: guard.path.to_string_lossy().into_owned(), identity: guard.identity.clone(), volume_id: guard.volume_id()?, writable, entries })
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Policy { KeepBoth, Skip, SkipIfExists, Replace }
    fn policy(value: &str) -> Result<Policy, String> {
        match value { "keepBoth" => Ok(Policy::KeepBoth), "skip" => Ok(Policy::Skip), "replace" => Ok(Policy::Replace), _ => Err("无效冲突策略，只支持保留两份、跳过或明确替换".into()) }
    }
    #[derive(Clone)]
    struct Existing { stamp: FileStamp, hash: Option<String> }
    #[derive(Clone)]
    pub(super) struct PlannedSource { pub(super) id: String, name: String, display_name: String, path: String, stamp: FileStamp, hash: String, existing: Option<Existing> }
    pub(super) struct Plan { pub(super) view: CopyPlan, created: Instant, target_path: String, target_identity: String, data_identity: String, pub(super) sources: Vec<PlannedSource> }
    pub(super) fn prune_plans(plans: &mut VecDeque<Plan>) { plans.retain(|plan| plan.created.elapsed() < PLAN_TTL); }
    struct BoundExisting { file: File, expected: Existing }
    struct Source { id: String, archive: ArchiveFile, name: String, policy: Policy, existing: Option<BoundExisting> }
    pub(super) struct Prepared { target: DirectoryGuard, protected: ProtectedRoot, sources: Vec<Source>, reserved: HashSet<String>, pub(super) volumes: Vec<String>, _activity: devices::Activity }
    impl Prepared { pub(super) fn source_count(&self) -> u64 { self.sources.len() as u64 } }

    fn checked_sources(sources: Vec<ExportSource>) -> Result<Vec<ExportSource>, String> {
        if sources.is_empty() || sources.len() > MAX_COPY_SOURCES { return Err("每批请选择 1–1000 份已归档资料".into()); }
        let mut seen = HashSet::new(); let mut result = Vec::new();
        for source in sources {
            if source.id.is_empty() || source.id.len() > 128 || source.id.chars().any(char::is_control) { return Err("归档资料 ID 无效".into()); }
            if seen.insert(source.id.clone()) { result.push(source); }
        }
        Ok(result)
    }
    fn archive_source(source: &ExportSource, protected: &ProtectedRoot, cache: &mut HashMap<PathBuf, Arc<DirectoryGuard>>) -> Result<ArchiveFile, String> {
        let path = protected.source_path(&source.path)?;
        let parent = path.parent().ok_or("归档来源没有父目录")?.to_path_buf();
        let guard = if let Some(guard) = cache.get(&parent) { guard.clone() } else {
            let guard = Arc::new(DirectoryGuard::open_internal(parent.to_str().ok_or("归档目录编码无效")?)?);
            cache.insert(parent, guard.clone()); guard
        };
        protected.open_archive(&path, guard)
    }
    fn batch_name(base: &str, reserved: &mut HashSet<String>, limit: usize) -> Result<String, String> {
        for ordinal in 0..MAX_NAME_ATTEMPTS {
            let name = conflict_name(base, ordinal, limit)?;
            if reserved.insert(name.to_lowercase()) { return Ok(name); }
        }
        Err("本批次重名超过 10000，无法分配安全名称".into())
    }
    fn existing_snapshot(target: &DirectoryGuard, name: &str) -> Result<Option<Existing>, String> {
        let metadata_file = match target.open_entry(name, false) {
            Ok(file) => file, Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("无法确认已有目标，未生成审批计划", error)),
        };
        let stamp = file_stamp(&metadata_file)?;
        if stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 { return Ok(Some(Existing { stamp, hash: None })); }
        if stamp.links != 1 { return Err("已有目标是硬链接，不能生成可能修改它的审批计划".into()); }
        drop(metadata_file);
        let mut file = target.open_entry(name, true).map_err(|error| io_error("已有文件正在写入或不可读，无法预览冲突", error))?;
        if file_stamp(&file)? != stamp { return Err("已有文件在预览期间变化，请重新预览".into()); }
        let hash = file_hash(&mut file)?;
        if file_stamp(&file)? != stamp { return Err("已有文件在摘要校验期间变化，请重新预览".into()); }
        Ok(Some(Existing { stamp, hash: Some(hash) }))
    }
    fn same_existing(a: &Option<Existing>, b: &Option<Existing>) -> bool {
        match (a, b) { (None, None) => true, (Some(a), Some(b)) => a.stamp == b.stamp && a.hash == b.hash, _ => false }
    }
    fn active_volumes(target: &DirectoryGuard, protected: &ProtectedRoot) -> Result<(Vec<String>, devices::Activity), String> {
        let volumes = devices::related_volumes(&[target.volume_root()?, protected.volume_root()?])?;
        let activity = devices::begin_activity(&volumes)?;
        Ok((volumes, activity))
    }
    pub(super) fn preview(sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<Plan, String> {
        let sources = checked_sources(sources)?; policy(&request.conflict)?;
        let protected = protected_root(data_root)?;
        let target = selected_target(&request.target_path, &request.target_identity, &protected, true)?;
        let (_volumes, _activity) = active_volumes(&target, &protected)?;
        let mut cache = HashMap::new(); let mut reserved = HashSet::new(); let mut planned = Vec::new(); let mut items = Vec::new();
        for source in sources {
            let mut archive = archive_source(&source, &protected, &mut cache)?;
            let name = batch_name(&safe_name(&source.display_name, target.name_limit())?, &mut reserved, target.name_limit())?;
            target.child(&name)?;
            let path = final_path(&archive.file)?;
            let hash = file_hash(&mut archive.file)?;
            if file_stamp(&archive.file)? != archive.stamp { return Err("归档在冲突预览期间变化，请重新获取资料租约".into()); }
            let existing = existing_snapshot(&target, &name)?;
            items.push(ConflictItem { source_id: source.id.clone(), source_name: source.display_name.clone(), target_name: name.clone(), exists: existing.is_some(), existing_size: existing.as_ref().map(|existing| existing.stamp.size) });
            planned.push(PlannedSource { id: source.id, name, display_name: source.display_name, path, stamp: archive.stamp, hash, existing });
            // 不保留全批文件句柄；计划只保留身份/摘要和精确 IDs。
        }
        target.validate()?; protected.reject_target(&target)?;
        Ok(Plan { view: CopyPlan { id: unique_id(), items }, created: Instant::now(), target_path: request.target_path, target_identity: target.identity.clone(), data_identity: protected.identity().into(), sources: planned })
    }
    pub(super) fn prepare(sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<Prepared, String> {
        let sources = checked_sources(sources)?;
        let policy = match request.conflict.as_str() {
            "keepBoth" => Policy::KeepBoth, "skip" => Policy::SkipIfExists,
            "replace" => return Err("替换必须先预览冲突并逐项明确批准；start_copy 不接受未批准替换".into()),
            _ => return Err("冲突策略无效".into()),
        };
        let protected = protected_root(data_root)?;
        let target = selected_target(&request.target_path, &request.target_identity, &protected, true)?;
        let (volumes, activity) = active_volumes(&target, &protected)?;
        let mut cache = HashMap::new(); let mut reserved = HashSet::new(); let mut opened = Vec::new();
        for source in sources {
            let name = batch_name(&safe_name(&source.display_name, target.name_limit())?, &mut reserved, target.name_limit())?;
            target.child(&name)?;
            opened.push(Source { id: source.id.clone(), archive: archive_source(&source, &protected, &mut cache)?, name, policy, existing: None });
        }
        target.validate()?; protected.reject_target(&target)?;
        Ok(Prepared { target, protected, sources: opened, reserved, volumes, _activity: activity })
    }
    pub(super) fn prepare_plan(sources: Vec<ExportSource>, plan: Plan, resolutions: Vec<ConflictResolution>, data_root: Option<String>) -> Result<Prepared, String> {
        if plan.created.elapsed() >= PLAN_TTL { return Err("复制审批已超过 300 秒，请重新预览".into()); }
        let sources = checked_sources(sources)?;
        let mut supplied: HashMap<_, _> = sources.into_iter().map(|source| (source.id.clone(), source)).collect();
        if supplied.len() != plan.sources.len() || plan.sources.iter().any(|source| !supplied.contains_key(&source.id)) { return Err("批准后的资料选择必须与预览 IDs 完全一致，不能追加、遗漏或换批".into()); }
        if resolutions.len() > plan.sources.len() { return Err("冲突决策超过本批次文件数".into()); }
        let mut decisions = HashMap::new();
        for resolution in resolutions {
            if !supplied.contains_key(&resolution.source_id) || decisions.insert(resolution.source_id, policy(&resolution.policy)?).is_some() { return Err("冲突决策包含未知或重复来源 ID".into()); }
        }
        let protected = protected_root(data_root)?;
        if protected.identity() != plan.data_identity { return Err("持久目录在审批后变化，必须重新预览并重新获取租约".into()); }
        let target = selected_target(&plan.target_path, &plan.target_identity, &protected, true)?;
        let (volumes, activity) = active_volumes(&target, &protected)?;
        let reserved = plan.sources.iter().map(|source| source.name.to_lowercase()).collect();
        let mut cache = HashMap::new(); let mut opened = Vec::new();
        for expected in plan.sources {
            let source = supplied.remove(&expected.id).ok_or("审批资料缺失")?;
            if source.display_name != expected.display_name { return Err("归档显示名称在审批后变化，请重新预览".into()); }
            let mut archive = archive_source(&source, &protected, &mut cache)?;
            if let Some(difference) = archive.stamp.changes_since(&expected.stamp) {
                return Err(format!("资料“{}”在审批后变化（{difference}），整批未启动复制", expected.display_name));
            }
            if !same_path(&final_path(&archive.file)?, &expected.path) { return Err(format!("资料“{}”的归档最终路径在审批后变化，整批未启动复制", expected.display_name)); }
            if file_hash(&mut archive.file)? != expected.hash { return Err(format!("资料“{}”的 SHA-256 在审批后变化，整批未启动复制", expected.display_name)); }
            if let Some(difference) = file_stamp(&archive.file)?.changes_since(&expected.stamp) { return Err(format!("资料“{}”在审批摘要复核期间变化（{difference}），整批未启动复制", expected.display_name)); }
            let decision = decisions.get(&expected.id).copied().unwrap_or(Policy::KeepBoth);
            let mut bound = None;
            if decision == Policy::Replace && expected.existing.is_some() {
                reject_system(&target.volume_root()?)?;
                let existing = expected.existing.as_ref().ok_or("替换批准缺少已有文件快照")?;
                if existing.stamp.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_SYSTEM) != 0 || existing.stamp.links != 1 || existing.hash.is_none() { return Err("不能替换目录、只读/系统文件或硬链接；请选择保留两份或跳过".into()); }
                let mut file = target.open_bound_entry(&expected.name)?;
                if file_stamp(&file)? != existing.stamp || file_hash(&mut file)? != existing.hash.clone().unwrap_or_default() || file_stamp(&file)? != existing.stamp { return Err(format!("已有文件 {} 的身份、内容或变更时间在审批后变化，请重新预览", expected.name)); }
                // 此句柄拒绝其他写/删除，持续到备份改名、提交和清理/恢复结束。
                bound = Some(BoundExisting { file, expected: existing.clone() });
            } else if !same_existing(&existing_snapshot(&target, &expected.name)?, &expected.existing) { return Err(format!("目标 {} 在审批后变化，请重新预览冲突", expected.name)); }
            opened.push(Source { id: expected.id, archive, name: expected.name, policy: if decision == Policy::Replace && bound.is_none() { Policy::KeepBoth } else { decision }, existing: bound });
        }
        target.validate()?; protected.reject_target(&target)?;
        Ok(Prepared { target, protected, sources: opened, reserved, volumes, _activity: activity })
    }

    enum Failure { Cancelled, Error(String) }
    impl From<String> for Failure { fn from(error: String) -> Self { Self::Error(error) } }
    impl From<&str> for Failure { fn from(error: &str) -> Self { Self::Error(error.to_string()) } }
    fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> { if cancel.load(Ordering::Acquire) { Err(Failure::Cancelled) } else { Ok(()) } }
    fn cancelled_io(context: &str, error: io::Error, cancel: &AtomicBool) -> Failure { if cancel.load(Ordering::Acquire) { Failure::Cancelled } else { Failure::Error(io_error(context, error)) } }
    fn write_chunk(file: &mut File, bytes: &[u8], cancel: &AtomicBool) -> Result<(), Failure> {
        let mut written = 0;
        while written < bytes.len() {
            check_cancel(cancel)?;
            match file.write(&bytes[written..]) {
                Ok(0) => return Err(Failure::Error("导出暂存未能继续写入".into())), Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(cancelled_io("写入导出暂存失败，可能空间不足或设备已移除", error, cancel)),
            }
        }
        Ok(())
    }
    struct Publisher { last: Instant }
    impl Publisher {
        fn publish(&mut self, registry: &Arc<Mutex<Registry>>, current: &CopyJob) -> Result<(), Failure> {
            if self.last.elapsed() < SNAPSHOT_INTERVAL { return Ok(()); }
            let mut registry = registry.lock().map_err(|_| Failure::Error("更新复制进度失败".into()))?;
            *registry.jobs.iter_mut().find(|job| job.id == current.id).ok_or_else(|| Failure::Error("复制记录缺失".into()))? = current.clone();
            self.last = Instant::now(); Ok(())
        }
    }
    pub(super) fn run(mut prepared: Prepared, mut current: CopyJob, cancel: Arc<AtomicBool>, registry: Arc<Mutex<Registry>>) {
        let mut publisher = Publisher { last: Instant::now() };
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Failure> {
            for source in &mut prepared.sources {
                check_cancel(&cancel)?; prepared.target.validate()?; prepared.protected.reject_target(&prepared.target)?;
                match copy_one(&prepared.target, &prepared.protected, &prepared.reserved, source, &cancel, &mut current, &mut publisher, &registry) {
                    Ok(skipped) => { current.processed += 1; if skipped { current.skipped += 1; } }
                    Err(Failure::Cancelled) => return Err(Failure::Cancelled),
                    Err(Failure::Error(error)) => {
                        current.processed += 1;
                        let remaining = current.total.saturating_sub(current.processed);
                        return Err(Failure::Error(format!("资料“{}”：{error}；后续 {remaining} 份未继续处理", source.name)));
                    }
                }
                publisher.publish(&registry, &current)?;
            }
            check_cancel(&cancel)?; prepared.target.validate()?; prepared.protected.reject_target(&prepared.target)?; Ok(())
        }));
        match result {
            Ok(Ok(())) => { current.state = "completed".into(); current.error = None; }
            Ok(Err(Failure::Cancelled)) => { current.state = "cancelled".into(); current.error = None; }
            Ok(Err(Failure::Error(error))) => { current.state = "failed".into(); current.error = Some(error); }
            Err(_) => { current.state = "failed".into(); current.error = Some("复制线程异常结束，请检查目录恢复记录；未确认文件不记成功".into()); }
        }
        // 先释放所有源/目标/备份句柄和设备活动，再发布结束状态。
        drop(prepared); finish_snapshot(&registry, &current);
    }
    fn create_temp(target: &DirectoryGuard, job_id: &str) -> Result<OwnedTemp, Failure> {
        for _ in 0..MAX_TEMP_ATTEMPTS {
            let name = format!(".celesvault-{job_id}-{:x}.tmp", SEQUENCE.fetch_add(1, Ordering::Relaxed));
            match OwnedTemp::create(target, &name) {
                Ok(temp) => return Ok(temp), Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("创建导出暂存失败", error).into()),
            }
        }
        Err(Failure::Error("自有暂存名称连续冲突，未删除或覆盖任何已有文件".into()))
    }
    fn copy_one(target: &DirectoryGuard, protected: &ProtectedRoot, reserved: &HashSet<String>, source: &mut Source, cancel: &AtomicBool, current: &mut CopyJob, publisher: &mut Publisher, registry: &Arc<Mutex<Registry>>) -> Result<bool, Failure> {
        if source.policy == Policy::Skip || (source.policy == Policy::SkipIfExists && target.entry_exists(&source.name)?) { target.validate()?; return Ok(true); }
        check_cancel(cancel)?;
        if file_stamp(&source.archive.file)? != source.archive.stamp { return Err("归档句柄在任务开始前变化，未导出".to_string().into()); }
        let mut temp = create_temp(target, &current.id)?;
        let result = (|| -> Result<bool, Failure> {
            temp.verify()?; target.validate()?;
            let mut buffer = vec![0u8; CHUNK_SIZE]; let mut copied_bytes = 0u64;
            let mut copied_hash = Sha256::new();
            loop {
                check_cancel(cancel)?;
                let count = match source.archive.file.read(&mut buffer) {
                    Ok(count) => count, Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(cancelled_io("读取归档来源失败", error, cancel)),
                };
                if count == 0 { break; }
                write_chunk(&mut temp.file, &buffer[..count], cancel)?;
                copied_hash.update(&buffer[..count]);
                copied_bytes = copied_bytes.checked_add(count as u64).ok_or_else(|| Failure::Error("复制字节计数溢出".into()))?;
                if copied_bytes > source.archive.stamp.size { return Err("归档大小在复制期间变化".to_string().into()); }
                publisher.publish(registry, current)?;
            }
            if copied_bytes != source.archive.stamp.size || file_stamp(&source.archive.file)? != source.archive.stamp { return Err("归档在读取期间发生变化，未提交导出".to_string().into()); }
            check_cancel(cancel)?;
            temp.file.flush().map_err(|error| cancelled_io("刷新导出暂存失败", error, cancel))?;
            temp.file.sync_all().map_err(|error| cancelled_io("同步导出暂存失败", error, cancel))?;
            check_cancel(cancel)?; temp.verify()?;
            let copied_hash = hex(&copied_hash.finalize());
            let staged_stamp = file_stamp(&temp.file)?;
            if file_hash(&mut temp.file)? != copied_hash || file_stamp(&temp.file)? != staged_stamp {
                return Err("导出暂存摘要或元数据与复制内容不一致，未提交".into());
            }
            check_cancel(cancel)?;
            if source.policy == Policy::Replace {
                replace_one(target, protected, source, &mut temp, copied_bytes, &copied_hash, cancel, current)?;
                return Ok(false);
            }
            for ordinal in 0..MAX_NAME_ATTEMPTS {
                check_cancel(cancel)?; target.validate()?; protected.reject_target(target)?;
                let name = conflict_name(&source.name, ordinal, target.name_limit())?;
                if ordinal != 0 && reserved.contains(&name.to_lowercase()) { continue; }
                if target.entry_exists(&name)? { if source.policy == Policy::SkipIfExists { return Ok(true); } continue; }
                temp.verify()?;
                match temp.publish(target, &name) {
                    Ok(()) => {
                        temp.verify_published(target, &name, copied_bytes, &copied_hash)?;
                        temp.file.sync_all().map_err(|error| io_error("已提交导出的磁盘同步失败，请检查设备", error))?;
                        target.validate()?;
                        current.copied += 1;
                        return Ok(false);
                    }
                    Err(error) => {
                        if matches!(error.raw_os_error(), Some(80) | Some(183)) || target.entry_exists(&name)? {
                            if source.policy == Policy::SkipIfExists { return Ok(true); } continue;
                        }
                        return Err(io_error("按目标目录句柄提交失败，未回退到路径覆盖", error).into());
                    }
                }
            }
            Err("重名数量超过 10000，未覆盖目标".to_string().into())
        })();
        if let Err(cleanup) = temp.cleanup() {
            return Err(Failure::Error(match result { Err(Failure::Error(error)) => format!("{error}；{cleanup}"), Err(Failure::Cancelled) => format!("复制已取消；{cleanup}"), _ => cleanup }));
        }
        result
    }

    #[derive(Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Intent {
        version: u32, id: String, kind: String, directory_identity: String, volume_id: String,
        original_name: String, held_name: String, old_stamp: FileStamp, old_hash: Option<String>,
        new_key: Option<FileKey>, staging_name: Option<String>, new_size: Option<u64>, new_hash: Option<String>, source_id: Option<String>,
    }
    fn intent_name(id: &str) -> String { format!(".celesvault-{id}.intent.json") }
    fn marker_name(id: &str) -> String { format!(".celesvault-{id}.complete.json") }
    fn persist_record(target: &DirectoryGuard, name: &str, record: &Intent) -> Result<OwnedTemp, String> {
        let bytes = serde_json::to_vec(record).map_err(|_| "编码文件恢复意图失败")?;
        if bytes.len() as u64 > MAX_INTENT_BYTES { return Err("文件恢复意图过长".into()); }
        let mut file = OwnedTemp::create(target, name).map_err(|error| io_error("创建文件恢复意图失败，未移动旧文件", error))?;
        file.file.write_all(&bytes).map_err(|error| io_error("写入文件恢复意图失败", error))?;
        file.file.sync_all().map_err(|error| io_error("同步文件恢复意图失败，未移动旧文件", error))?;
        file.verify()?;
        // 从此即便 panic/断电也保留意图；正常结束显式清理按已知句柄执行。
        file.preserve(); Ok(file)
    }
    fn remove_known_record(record: &mut OwnedTemp) -> Result<(), String> {
        record.verify()?;
        delete_handle(&record.file).map_err(|error| io_error("操作已提交，但自有恢复记录清理失败", error))
    }
    fn delete_handle(file: &File) -> Result<(), io::Error> {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe { SetFileInformationByHandle(file.as_raw_handle().cast(), FileDispositionInfo, ptr::from_ref(&disposition).cast(), size_of::<FILE_DISPOSITION_INFO>() as u32) } == 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
    fn verify_held(file: &mut File, target: &DirectoryGuard, record: &Intent) -> Result<(), String> {
        let stamp = file_stamp(file)?;
        if stamp.key != record.old_stamp.key || stamp.size != record.old_stamp.size || stamp.modified != record.old_stamp.modified
            || stamp.attributes != record.old_stamp.attributes || stamp.links != record.old_stamp.links
            || !same_path(&final_path(file)?, &target.child(&record.held_name)?.to_string_lossy()) { return Err("恢复文件已变化或不是已批准的同一来源，拒绝修改".into()); }
        if let Some(expected) = &record.old_hash { if file_hash(file)? != *expected { return Err("恢复备份内容不匹配，拒绝修改".into()); } }
        if file_stamp(file)? != stamp { return Err("恢复备份在摘要核验期间变化，拒绝修改".into()); }
        Ok(())
    }
    fn restore_bound(file: &mut File, target: &DirectoryGuard, record: &Intent) -> Result<(), String> {
        target.validate()?; verify_held(file, target, record)?;
        if target.entry_exists(&record.original_name)? { return Err("原名称已被新文件占用，旧备份保持原处，不覆盖或删除新来文件".into()); }
        rename_no_replace(file, target, &record.original_name).map_err(|error| io_error("按旧备份句柄恢复原名失败，备份保留", error))?;
        if file_stamp(file)?.key != record.old_stamp.key || !same_path(&final_path(file)?, &target.child(&record.original_name)?.to_string_lossy()) { return Err("旧文件已恢复但最终身份复核失败，请检查目录".into()); }
        target.validate()
    }
    fn retained_error(target: &DirectoryGuard, record: &Intent, context: &str) -> String {
        format!("{context}；旧文件保留于 {}（卷 {}），请使用目录恢复操作；不会覆盖原名处的新文件", target.child(&record.held_name).map_or_else(|_| record.held_name.clone(), |path| path.to_string_lossy().into_owned()), record.volume_id)
    }
    fn replace_one(target: &DirectoryGuard, protected: &ProtectedRoot, source: &mut Source, temp: &mut OwnedTemp, size: u64, expected_hash: &str, cancel: &AtomicBool, current: &mut CopyJob) -> Result<(), Failure> {
        let old = source.existing.as_mut().ok_or_else(|| Failure::Error("替换缺少已批准旧目标句柄".into()))?;
        check_cancel(cancel)?; target.validate()?; protected.reject_target(target)?;
        if file_stamp(&old.file)? != old.expected.stamp || file_hash(&mut old.file)? != old.expected.hash.clone().unwrap_or_default() || file_stamp(&old.file)? != old.expected.stamp { return Err("已有目标在提交前变化，拒绝替换且不转用保留两份".to_string().into()); }
        let id = unique_id();
        let staging_path = PathBuf::from(final_path(&temp.file)?);
        let staging_name = staging_path.file_name().and_then(|name| name.to_str()).ok_or("自有暂存名称编码无效")?.to_string();
        let staged_hash = file_hash(&mut temp.file)?;
        let record = Intent { version: 1, id: id.clone(), kind: "replace".into(), directory_identity: target.identity.clone(), volume_id: target.volume_id()?, original_name: source.name.clone(), held_name: format!(".celesvault-{id}.backup"), old_stamp: old.expected.stamp.clone(), old_hash: old.expected.hash.clone(), new_key: Some(temp.key().clone()), staging_name: Some(staging_name), new_size: Some(size), new_hash: Some(staged_hash), source_id: Some(source.id.clone()) };
        let mut intent = persist_record(target, &intent_name(&id), &record)?;
        if let Err(cancelled) = check_cancel(cancel) { let _ = remove_known_record(&mut intent); return Err(cancelled); }
        target.validate()?; temp.verify()?;
        if file_stamp(&old.file)? != old.expected.stamp { let _ = remove_known_record(&mut intent); return Err("已有目标元数据在最后提交前变化，请重新预览".to_string().into()); }
        // 不是 pre-delete：旧目标按其已批准句柄改名到唯一备份，严格不覆盖。
        // 原名空窗若被外部进程占用，后续 publish/恢复均拒绝替换那份新文件。
        if let Err(error) = rename_no_replace(&old.file, target, &record.held_name) {
            let _ = remove_known_record(&mut intent);
            return Err(io_error("固定旧目标改名到恢复备份失败，未替换", error).into());
        }
        let mut backup_deleted = false;
        let result = (|| -> Result<(), Failure> {
            check_cancel(cancel)?; verify_held(&mut old.file, target, &record)?;
            protected.reject_target(target)?; temp.verify()?;
            temp.publish(target, &source.name).map_err(|error| io_error("替换提交发生名称冲突或设备错误，不覆盖新来文件", error))?;
            temp.verify_published(target, &source.name, size, expected_hash)?;
            temp.file.sync_all().map_err(|error| io_error("同步新导出失败，旧备份仍保留", error))?;
            target.validate()?; protected.reject_target(target)?; verify_held(&mut old.file, target, &record)?;
            current.copied += 1;
            // durable 完成标记先落盘，才按已验证的旧句柄删除本次备份。
            let mut marker = persist_record(target, &marker_name(&id), &record)?;
            verify_held(&mut old.file, target, &record)?;
            delete_handle(&old.file).map_err(|error| io_error("新文件已提交但旧备份清理失败，备份保留", error))?;
            backup_deleted = true;
            remove_known_record(&mut intent)?; remove_known_record(&mut marker)?;
            Ok(())
        })();
        if let Err(error) = result {
            let reason = match error { Failure::Cancelled => "替换已取消".to_string(), Failure::Error(error) => error };
            if backup_deleted { return Err(Failure::Error(format!("{reason}；新文件已同步提交，旧备份已按明确替换清理，仅恢复记录清理未完成"))); }
            // 新文件已经发布时不撤销它；原名未占用才尝试恢复旧目标。
            match restore_bound(&mut old.file, target, &record) {
                Ok(()) => { let _ = remove_known_record(&mut intent); Err(Failure::Error(format!("{reason}；已恢复旧文件，未覆盖其他目标"))) }
                Err(restore) => Err(Failure::Error(retained_error(target, &record, &format!("{reason}；{restore}")))),
            }
        } else { Ok(()) }
    }

    pub(super) struct BrowserRead { _file: File, _parents: DirectoryGuard, _protected: ProtectedRoot, _activity: devices::Activity }
    pub(super) fn lease_browser_entry(path: String, identity: String, data_root: Option<String>) -> Result<BrowserEntryLease, String> {
        if identity.is_empty() || identity.len() > 32_768 { return Err("缺少有效浏览项身份，请刷新后打开或定位".into()); }
        let protected = protected_root(data_root)?;
        let path = local_path(&path)?;
        let text = path.to_str().ok_or("浏览项路径编码无效")?;
        let (parents, file, final_name) = if text.len() == 3 {
            let guard = DirectoryGuard::open(text)?;
            if guard.identity != identity { return Err("驱动器身份已变化，未请求系统打开".into()); }
            protected.reject_target(&guard)?;
            let file = guard.file().try_clone().map_err(|error| io_error("保留浏览根句柄失败", error))?;
            let name = guard.final_path.clone();
            (guard, file, name)
        } else {
            let parent = path.parent().and_then(|path| path.to_str()).ok_or("浏览项没有安全父目录")?;
            let parents = DirectoryGuard::open(parent)?;
            protected.reject_target(&parents)?;
            let name = path.file_name().and_then(|name| name.to_str()).ok_or("浏览项名称编码无效")?;
            if internal_hidden(name) { return Err("不通过通用浏览打开暂存或系统内部项".into()); }
            let real = parents.entry_path(name)?;
            if protected.hides_entry(&real) { return Err("不通过通用浏览打开持久目录内部资料，请使用归档资料入口".into()); }
            let file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(&real).map_err(|error| io_error("只读固定浏览项失败，文件可能正在变化", error))?;
            let stamp = file_stamp(&file)?; let final_name = final_path(&file)?;
            if stamp.attributes & FILE_ATTRIBUTE_SYSTEM != 0 || (stamp.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 && stamp.links != 1)
                || !same_path(&final_name, &real.to_string_lossy()) || parents.entry_identity(&stamp, &final_name)? != identity { return Err("浏览项身份已变化或属于受保护项，未请求系统打开".into()); }
            if file_stamp(&file)? != stamp { return Err("浏览项在校验期间变化，请刷新后重试".into()); }
            (parents, file, final_name)
        };
        parents.validate()?;
        let activity = devices::begin_activity(&devices::related_volumes(&[parents.volume_root()?])?)?;
        Ok(BrowserEntryLease { path: PathBuf::from(final_name), _entry: BrowserRead { _file: file, _parents: parents, _protected: protected, _activity: activity } })
    }

    pub(super) fn create_folder(path: String, identity: String, name: String, data_root: Option<String>) -> Result<(), String> {
        let protected = protected_root(data_root)?;
        let guard = DirectoryGuard::for_create(&path)?;
        if guard.identity != identity { return Err("目录身份已变化，请刷新后新建文件夹".into()); }
        protected.reject_target(&guard)?; reject_system(&guard.volume_root()?)?; user_name(&name, guard.name_limit())?;
        let child = guard.child(&name)?;
        if protected.hides_entry(&child) { return Err("不能在持久目录内部新建浏览文件夹".into()); }
        let _activity = devices::begin_activity(&devices::related_volumes(&[guard.volume_root()?])?)?;
        guard.validate()?;
        fs::create_dir(&child).map_err(|error| io_error("新建文件夹失败；已有同名项不会覆盖", error))?;
        let file = guard.open_entry(&name, false).map_err(|error| io_error("文件夹已新建但身份无法核验，请刷新", error))?;
        if file_stamp(&file)?.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 { return Err("新建后目录项已变化，请刷新，不会修改该项".into()); }
        guard.validate()?; protected.reject_target(&guard)
    }
    pub(super) fn rename_entry(path: String, identity: String, name: String, data_root: Option<String>) -> Result<(), String> {
        let protected = protected_root(data_root)?; let entry = EntryGuard::open(&path, &identity, true)?;
        if internal_hidden(entry.display_path.file_name().and_then(|name| name.to_str()).unwrap_or("")) || entry.stamp.attributes & FILE_ATTRIBUTE_SYSTEM != 0 { return Err("不能通过通用重命名修改暂存或系统内部文件".into()); }
        protected.reject_mutation(&entry.final_path)?; reject_system(&entry.parents.volume_root()?)?;
        let destination = entry.parents.child(&name)?; protected.reject_mutation(&destination.to_string_lossy())?;
        let _activity = devices::begin_activity(&devices::related_volumes(&[entry.parents.volume_root()?])?)?;
        entry.validate()?; entry.rename(&name)
    }

    fn read_intent(target: &DirectoryGuard, name: &str) -> Result<(Intent, File), String> {
        let mut file = target.open_bound_entry(name)?; let stamp = file_stamp(&file)?;
        if stamp.links != 1 || stamp.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 || stamp.size > MAX_INTENT_BYTES { return Err("恢复意图不是有界普通文件".into()); }
        let mut bytes = Vec::new(); (&mut file).take(MAX_INTENT_BYTES + 1).read_to_end(&mut bytes).map_err(|error| io_error("读取恢复意图失败", error))?;
        if file_stamp(&file)? != stamp { return Err("恢复意图在读取期间变化".into()); }
        let record: Intent = serde_json::from_slice(&bytes).map_err(|_| "恢复意图内容无效，不自动执行")?;
        if record.version != 1 || !valid_recovery_id(&record.id) || intent_name(&record.id) != name
            || record.directory_identity != target.identity || record.volume_id != target.volume_id()?
            || !matches!(record.kind.as_str(), "replace" | "recycle") { return Err("恢复意图与当前目录/卷代际不匹配".into()); }
        user_name(&record.original_name, target.name_limit())?;
        let suffix = if record.kind == "replace" { "backup" } else { "recycle" };
        if record.held_name != format!(".celesvault-{}.{suffix}", record.id) { return Err("恢复备份名称不是本意图的直接子项".into()); }
        target.child(&record.held_name)?;
        Ok((record, file))
    }
    fn valid_recovery_id(id: &str) -> bool { !id.is_empty() && id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') }
    pub(super) fn file_recoveries(path: String, identity: String, data_root: Option<String>) -> Result<Vec<FileRecovery>, String> {
        let protected = protected_root(data_root)?; let target = selected_target(&path, &identity, &protected, false)?;
        let mut rows = Vec::new();
        for (index, entry) in fs::read_dir(&target.final_path).map_err(|error| io_error("读取目录恢复记录失败", error))?.enumerate() {
            if index >= MAX_DIRECTORY_ENTRIES { return Err("目录超过 10000 项，不返回不完整恢复列表".into()); }
            let entry = entry.map_err(|error| io_error("枚举恢复记录失败", error))?;
            let name = entry.file_name().into_string().map_err(|_| "恢复记录名称编码无效")?;
            if !name.starts_with(".celesvault-") || !name.ends_with(".intent.json") { continue; }
            if rows.len() >= 256 { return Err("本目录恢复记录超过 256，请先处理现有记录".into()); }
            let (record, _intent_file) = read_intent(&target, &name)?;
            let check = (|| -> Result<(), String> {
                let mut held = target.open_bound_entry(&record.held_name)?;
                verify_held(&mut held, &target, &record)?;
                if target.entry_exists(&record.original_name)? { return Err("原名称已有文件，恢复不会覆盖；请先明确处理原名项".into()); }
                Ok(())
            })();
            rows.push(FileRecovery { id: record.id, kind: record.kind, original_name: record.original_name, held_path: target.child(&record.held_name)?.to_string_lossy().into_owned(), held_name: record.held_name, can_restore: check.is_ok(), error: check.err() });
        }
        target.validate()?; Ok(rows)
    }
    pub(super) fn restore_file_recovery(path: String, identity: String, recovery_id: String, data_root: Option<String>) -> Result<(), String> {
        if !valid_recovery_id(&recovery_id) { return Err("恢复记录 ID 无效".into()); }
        let protected = protected_root(data_root)?; let target = selected_target(&path, &identity, &protected, true)?;
        reject_system(&target.volume_root()?)?;
        let _activity = devices::begin_activity(&devices::related_volumes(&[target.volume_root()?])?)?;
        let (record, intent_file) = read_intent(&target, &intent_name(&recovery_id))?;
        protected.reject_mutation(&target.child(&record.original_name)?.to_string_lossy())?;
        let mut held = target.open_bound_entry(&record.held_name)?;
        restore_bound(&mut held, &target, &record)?;
        // 恢复记录是磁盘数据而非删除授权；只改为 resolved，不自动删除其他暂存。
        rename_no_replace(&intent_file, &target, &format!(".celesvault-{recovery_id}.resolved.json")).map_err(|error| io_error("旧文件已恢复，但恢复记录标记失败，请刷新", error))?;
        target.validate()
    }

    pub(super) fn recycle_entry(path: String, identity: String, data_root: Option<String>) -> Result<(), String> {
        let protected = protected_root(data_root)?; let mut entry = EntryGuard::open(&path, &identity, false)?;
        let original = entry.display_path.file_name().and_then(|name| name.to_str()).ok_or("回收文件名称编码无效")?;
        if internal_hidden(original) || entry.stamp.attributes & FILE_ATTRIBUTE_SYSTEM != 0 { return Err("不能通过通用回收删除暂存或系统内部项".into()); }
        protected.reject_mutation(&entry.final_path)?; reject_system(&entry.parents.volume_root()?)?;
        devices::require_recycle_volume(&entry.parents.volume_root()?)?;
        let _activity = devices::begin_activity(&devices::related_volumes(&[entry.parents.volume_root()?])?)?;
        entry.validate()?;
        // 父链/GUID固定；叶允许 DELETE 共享使 Shell 可回收，但仍拒绝其他写入。
        // 即时校验完整来源身份，不承诺恶意同权限进程无法在 Shell 内部并发改名。
        if entry.stamp.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 { let _ = file_hash(&mut entry.file)?; }
        entry.validate()?; protected.reject_mutation(&entry.final_path)?;
        shell_recycle(&entry.final_path)?;
        entry.parents.validate()?;
        let recycled = final_path(&entry.file)?;
        let current = file_stamp(&entry.file)?;
        if current.key != entry.stamp.key || current.size != entry.stamp.size
            || devices::normalize_volume_identifier(&recycled)? != entry.parents.volume_root()?
            || !recycled.to_ascii_lowercase().contains("\\$recycle.bin\\") {
            return Err("系统操作返回后，未能核实同一来源句柄进入系统回收站；请检查系统回收站，不再重试删除".into());
        }
        if entry.parents.entry_exists(original)? { return Err("同一原件已回收，但原名称出现了新目录项；请刷新，不会处理新来文件".into()); }
        Ok(())
    }

    /// 最小 COM ABI 封装，仅使用 IFileOperation 的 recycle-only 删除。没有 ShellExecute、
    /// 进程启动、安装或文件关联操作。所有 COM 指针限于本次调用线程，并 RAII Release。
    fn shell_recycle(path: &str) -> Result<(), String> {
        // 使用短生命周期 STA 线程，避免复用的 Tauri blocking 线程已进入 MTA 时
        // 永久失败。调用方的来源/父链/活动租约一直持有到同步请求返回。
        let path = path.to_string();
        std::thread::Builder::new().name("celesvault-recycle-sta".into()).spawn(move || shell_recycle_sta(&path))
            .map_err(|error| io_error("启动系统回收 STA 线程失败，未回收原件", error))?
            .join().map_err(|_| "系统回收线程异常结束，请检查系统回收站，不重复删除".to_string())?
    }
    fn shell_recycle_sta(path: &str) -> Result<(), String> {
        use std::ffi::c_void;
        use windows_sys::{core::GUID, Win32::{System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED}, UI::Shell::{FileOperation, SHCreateItemFromParsingName, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_NORECURSEREPARSE, FOF_NO_CONNECTED_ELEMENTS, FOFX_RECYCLEONDELETE, FOFX_EARLYFAILURE, FOFX_NOCOPYHOOKS}}};
        #[repr(C)]
        struct UnknownVtable { query: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32, add_ref: unsafe extern "system" fn(*mut c_void) -> u32, release: unsafe extern "system" fn(*mut c_void) -> u32 }
        #[repr(C)]
        struct FileOperationVtable {
            unknown: UnknownVtable,
            advise: usize, unadvise: usize,
            set_flags: unsafe extern "system" fn(*mut c_void, u32) -> i32,
            progress_message: usize, progress_dialog: usize, properties: usize, owner: usize,
            apply_properties_item: usize, apply_properties_items: usize,
            rename_item: usize, rename_items: usize, move_item: usize, move_items: usize, copy_item: usize, copy_items: usize,
            delete_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
            delete_items: usize, new_item: usize,
            perform: unsafe extern "system" fn(*mut c_void) -> i32,
            aborted: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
        }
        struct ComPtr(*mut c_void);
        impl Drop for ComPtr { fn drop(&mut self) { if !self.0.is_null() { unsafe { let table = *(self.0 as *const *const UnknownVtable); ((*table).release)(self.0); } } } }
        struct Apartment;
        impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
        fn hr(code: i32, context: &str) -> Result<(), String> { if code < 0 { Err(format!("{context}（HRESULT 0x{:08x}），未改用永久删除", code as u32)) } else { Ok(()) } }
        // Native 命令由宿主 spawn_blocking 调用；若线程已是 MTA，返回明确错误，不更改公寓。
        hr(unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) }, "初始化系统回收接口失败")?;
        let _apartment = Apartment;
        let iid_operation = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);
        let iid_item = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
        let mut operation = ptr::null_mut();
        hr(unsafe { CoCreateInstance(&FileOperation, ptr::null_mut(), CLSCTX_INPROC_SERVER, &iid_operation, &mut operation) }, "创建系统回收操作失败")?;
        if operation.is_null() { return Err("系统回收接口返回空对象".into()); }
        let operation = ComPtr(operation); let mut item = ptr::null_mut();
        hr(unsafe { SHCreateItemFromParsingName(wide(path).as_ptr(), ptr::null_mut(), &iid_item, &mut item) }, "系统无法解析已固定卷 GUID 回收路径")?;
        if item.is_null() { return Err("系统回收来源对象为空".into()); }
        let item = ComPtr(item);
        let table = unsafe { *(operation.0 as *const *const FileOperationVtable) };
        let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_NORECURSEREPARSE | FOF_NO_CONNECTED_ELEMENTS | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE | FOFX_NOCOPYHOOKS;
        hr(unsafe { ((*table).set_flags)(operation.0, flags) }, "系统未接受仅回收策略")?;
        hr(unsafe { ((*table).delete_item)(operation.0, item.0, ptr::null_mut()) }, "登记系统回收来源失败")?;
        hr(unsafe { ((*table).perform)(operation.0) }, "系统回收失败，文件可能正在使用或回收站不可用")?;
        let mut aborted = 0;
        hr(unsafe { ((*table).aborted)(operation.0, &mut aborted) }, "无法确认系统回收完成状态")?;
        if aborted != 0 { return Err("系统回收被取消或未完整执行，不宣称成功".into()); }
        Ok(())
    }

    struct Event(OwnedHandle);
    impl Event {
        fn new() -> Result<Self, String> {
            let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
            if handle.is_null() { return Err(io_error("创建目录监听取消事件失败", io::Error::last_os_error())); }
            Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle.cast()) }))
        }
        fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE { self.0.as_raw_handle().cast() }
        fn set(&self) -> Result<(), String> { if unsafe { SetEvent(self.raw()) } == 0 { Err(io_error("设置目录监听取消事件失败", io::Error::last_os_error())) } else { Ok(()) } }
    }
    pub(super) struct WatchWorker { pub(super) handle: JoinHandle<()>, pub(super) volumes: Vec<String>, stop: Arc<AtomicBool>, wake: Arc<Event>, done: Receiver<()>, ready: Receiver<Result<(), String>> }
    impl WatchWorker {
        pub(super) fn wait_ready(&mut self) -> Result<(), String> {
            self.ready.recv_timeout(Duration::from_secs(2)).map_err(|_| "Windows 目录监听未在 2 秒内确认启动，不能假装已经监听".to_string())?
        }
        pub(super) fn stop(&mut self) -> Result<(), String> {
            self.stop.store(true, Ordering::Release); let wake = self.wake.set(); let sync_cancel = cancel_thread(&self.handle);
            if !self.handle.is_finished() {
                match self.done.recv_timeout(Duration::from_secs(2)) {
                    Ok(()) => (), Err(mpsc::RecvTimeoutError::Disconnected) if self.handle.is_finished() => (),
                    _ => return Err("目录监听已停止回调，但设备未在 2 秒内结束 I/O；保留句柄/缓冲，不启动替代监听或强杀线程".into()),
                }
            }
            wake?; sync_cancel?; Ok(())
        }
    }
    pub(super) fn start_watch(path: String, identity: String, data_root: Option<String>, callback: Arc<dyn Fn() + Send + Sync>, error: Arc<Mutex<Option<String>>>) -> Result<WatchWorker, String> {
        let protected = protected_root(data_root)?; let guard = selected_target(&path, &identity, &protected, false)?;
        let file = OpenOptions::new().access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
            .open(&guard.final_path).map_err(|failure| io_error("打开 Windows 目录监听失败", failure))?;
        if !same_path(&final_path(&file)?, &guard.final_path) { return Err("监听句柄与选中目录不匹配".into()); }
        let volumes = devices::related_volumes(&[guard.volume_root()?])?; let activity = devices::begin_activity(&volumes)?;
        let stop = Arc::new(AtomicBool::new(false)); let wake = Arc::new(Event::new()?);
        let worker_stop = stop.clone(); let worker_wake = wake.clone();
        let (done_send, done) = mpsc::sync_channel(1);
        let (ready_send, ready) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new().name("celesvault-directory-watch".into()).spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| watch_loop(guard, protected, file, worker_stop.clone(), worker_wake, callback.clone(), activity, &ready_send)));
            let failure = match outcome { Ok(Ok(())) => None, Ok(Err(error)) => Some(error), Err(_) => Some("目录监听线程异常结束，必须重新浏览选择".into()) };
            if let Some(failure) = failure {
                let _ = ready_send.try_send(Err(failure.clone()));
                *error.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(failure);
                if !worker_stop.load(Ordering::Acquire) { let _ = catch_unwind(AssertUnwindSafe(|| callback())); }
            } else { let _ = ready_send.try_send(Err("目录监听在启动确认前已取消".into())); }
            let _ = done_send.try_send(());
        }).map_err(|failure| io_error("启动目录监听线程失败", failure))?;
        Ok(WatchWorker { handle, volumes, stop, wake, done, ready })
    }
    fn drain_watch(file: &File, overlapped: *const OVERLAPPED, completion: &Event) {
        // CancelIoEx 是请求而非完成证明。直到 GetOverlappedResult 报终态，线程一直
        // 保留 File/OVERLAPPED/缓冲；公共 unwatch 仍只等待 2 秒并可返回未释放错误。
        unsafe { CancelIoEx(file.as_raw_handle().cast(), overlapped); }
        loop {
            let mut transferred = 0;
            if unsafe { GetOverlappedResult(file.as_raw_handle().cast(), overlapped, &mut transferred, 0) } != 0 { return; }
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(code) if code == ERROR_IO_INCOMPLETE as i32 || code == ERROR_IO_PENDING as i32) { return; }
            if unsafe { WaitForSingleObject(completion.raw(), 250) } != WAIT_TIMEOUT {
                // 驱动若错误地提前置事件，避免自旋；不强杀或释放仍在用的内存。
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    struct PendingWatch<'a> { file: &'a File, overlapped: *const OVERLAPPED, completion: &'a Event, active: bool }
    impl Drop for PendingWatch<'_> {
        fn drop(&mut self) { if self.active { drain_watch(self.file, self.overlapped, self.completion); } }
    }
    fn watch_loop(guard: DirectoryGuard, protected: ProtectedRoot, file: File, stop: Arc<AtomicBool>, wake: Arc<Event>, callback: Arc<dyn Fn() + Send + Sync>, _activity: devices::Activity, ready: &mpsc::SyncSender<Result<(), String>>) -> Result<(), String> {
        let completion = Event::new()?;
        let overlapped = Box::new(std::cell::UnsafeCell::new(OVERLAPPED::default()));
        let overlapped_ptr = overlapped.get();
        let mut buffer = vec![0u64; (32 * 1024) / 8];
        let mut dirty = false; let mut first = true;
        let mut last_callback = Instant::now(); let mut last_identity = Instant::now();
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_ATTRIBUTES | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;
        loop {
            if stop.load(Ordering::Acquire) { return Ok(()); }
            if unsafe { ResetEvent(completion.raw()) } == 0 { return Err(io_error("重置目录监听事件失败", io::Error::last_os_error())); }
            unsafe { *overlapped_ptr = OVERLAPPED::default(); (*overlapped_ptr).hEvent = completion.raw(); }
            // SAFETY: 输出对齐且容量精确；File、缓冲、Box<OVERLAPPED> 和事件均保持
            // 到该次 I/O 完成。仅监测当前目录，不递归扫描全盘或使用回调裸指针。
            let queued = unsafe { ReadDirectoryChangesW(file.as_raw_handle().cast(), buffer.as_mut_ptr().cast(), (buffer.len() * 8) as u32, 0, filter, ptr::null_mut(), overlapped_ptr, None) };
            if queued == 0 {
                let failure = io::Error::last_os_error();
                if failure.raw_os_error() != Some(ERROR_IO_PENDING as i32) { return Err(io_error("Windows 目录监听启动失败", failure)); }
            }
            // RAII 排空覆盖 panic 路径；flight 比缓冲/OVERLAPPED 后声明、先析构。
            let mut flight = PendingWatch { file: &file, overlapped: overlapped_ptr.cast_const(), completion: &completion, active: true };
            if first { let _ = ready.try_send(Ok(())); first = false; }
            let mut completed = false;
            while !completed {
                let handles = [wake.raw(), completion.raw()];
                let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, 200) };
                if stop.load(Ordering::Acquire) || wait == WAIT_OBJECT_0 {
                    drain_watch(&file, overlapped_ptr.cast_const(), &completion); return Ok(());
                }
                if wait != WAIT_OBJECT_0 + 1 && wait != WAIT_TIMEOUT {
                    let failure = io_error("等待目录监听失败", io::Error::last_os_error());
                    drain_watch(&file, overlapped_ptr.cast_const(), &completion); return Err(failure);
                }
                if wait == WAIT_OBJECT_0 + 1 {
                    let mut transferred = 0;
                    if unsafe { GetOverlappedResult(file.as_raw_handle().cast(), overlapped_ptr.cast_const(), &mut transferred, 0) } == 0 {
                        let failure = io::Error::last_os_error();
                        if failure.raw_os_error() == Some(ERROR_NOTIFY_ENUM_DIR as i32) { dirty = true; completed = true; flight.active = false; }
                        else if failure.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) { std::thread::sleep(Duration::from_millis(10)); }
                        else {
                            drain_watch(&file, overlapped_ptr.cast_const(), &completion);
                            if stop.load(Ordering::Acquire) && failure.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) { return Ok(()); }
                            return Err(io_error("目录监听 I/O 失败，卷或目录可能已移除", failure));
                        }
                    } else { dirty = true; completed = true; flight.active = false; } // transferred=0 也是溢出刷新信号。
                }
                if last_identity.elapsed() >= Duration::from_secs(1) {
                    if let Err(failure) = guard.validate().and_then(|_| protected.reject_target(&guard)) {
                        if !completed { drain_watch(&file, overlapped_ptr.cast_const(), &completion); }
                        return Err(failure);
                    }
                    last_identity = Instant::now();
                }
                if dirty && last_callback.elapsed() >= SNAPSHOT_INTERVAL && !stop.load(Ordering::Acquire) {
                    if catch_unwind(AssertUnwindSafe(|| callback())).is_err() {
                        if !completed { drain_watch(&file, overlapped_ptr.cast_const(), &completion); }
                        return Err("目录监听回调异常，监听已停止".into());
                    }
                    dirty = false; last_callback = Instant::now();
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn batch_names_preserve_extension_and_are_unique() {
            let mut reserved = HashSet::new();
            assert_eq!(batch_name("课件.pptx", &mut reserved, 180).unwrap(), "课件.pptx");
            assert_eq!(batch_name("课件.pptx", &mut reserved, 180).unwrap(), "课件 (2).pptx");
            assert_eq!(batch_name("课件.PPTX", &mut reserved, 180).unwrap(), "课件 (3).PPTX");
        }
        #[test]
        fn policy_and_recovery_ids_are_strict() {
            assert!(policy("replace").is_ok()); assert!(policy("replaceAll").is_err());
            assert!(!valid_recovery_id("../other")); assert!(!valid_recovery_id("x:ads"));
            assert!(valid_recovery_id("export-abc-123"));
        }
    }
}
