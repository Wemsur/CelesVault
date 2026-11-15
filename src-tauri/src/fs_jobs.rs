// SPDX-License-Identifier: GPL-3.0-only
//! 阶段 2 真实目录浏览与归档导出，没有 Tauri、网络、移动或系统弹出依赖。
//! 参考 Sigma dir_reader/{types,read,drives} 与 copy_move_job/file_operations 的
//! 目录视图、保留两份/跳过和取消设计；没有复制其业务模块或无界事件通道。
//!
//! 来源由宿主先锁定归档再按 ID 校验摘要，并持有校验租约直到 start_copy 返回；
//! prepare 在返回前接手全批只读文件句柄，避免校验后被修改。ExportSource 不实现 Deserialize。
//! total/processed/copied/skipped 都是去重后的文件数，不是字节数。
//! 至多一个后台线程，保留至多 32 个结束任务；宿主轮询快照，不发送无界事件。
//! shutdown 必须在 app.exit 前调用：取消并 join 后才释放目标/归档句柄。
//! 崩溃时 .celesvault-* 暂存可能遗留，只隐藏而不自动扫描清理；可信恢复后置。

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread::JoinHandle,
};

const MAX_FINISHED_JOBS: usize = 32;
const UNSUPPORTED: &str = "真实目录浏览与导出仅支持 Windows 本地盘，未回退到模拟模式";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub root: String,
    pub name: String,
    /// 本次挂载的卷/根身份，不是永久物理 USB ID。
    pub identity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryView {
    pub path: String,
    pub identity: String,
    pub entries: Vec<DirectoryEntry>,
}

/// 只由 Rust 宿主构造；不得为前端增加任意来源路径命令。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSource {
    pub id: String,
    pub display_name: String,
    pub path: PathBuf,
}

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
    fn default() -> Self {
        Self { target_path: String::new(), target_identity: String::new(), conflict: default_conflict() }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyJob {
    pub id: String,
    /// running / completed / failed / cancelled。
    pub state: String,
    pub total: u64,
    pub processed: u64,
    pub copied: u64,
    pub skipped: u64,
    pub error: Option<String>,
}

#[derive(Default)]
struct Registry { jobs: VecDeque<CopyJob> }

struct Worker {
    id: String,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Worker {
    fn request_cancel(&self) -> Result<(), String> {
        // 先设置标记，确保被中断的 read/write 不会重试下一块或误记为普通复制失败。
        self.cancel.store(true, Ordering::Release);
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Foundation::ERROR_NOT_FOUND, System::IO::CancelSynchronousIo};
            if self.handle.is_finished() { return Ok(()); }
            // SAFETY: JoinHandle 在整个同步调用期间仍由 Worker 持有；cancel 持有
            // worker 锁，shutdown 则独占取出的 Worker。没有 join/关闭或复用裸句柄。
            if unsafe { CancelSynchronousIo(self.handle.as_raw_handle().cast()) } == 0 {
                let error = std::io::Error::last_os_error();
                // 无正在执行的同步 I/O 很正常，下一块仍会检查取消标记。
                if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                    return Err(crate::path_guard::io_error(
                        "已请求取消，但中断设备同步 I/O 失败；仍需等待设备返回，不会强杀线程", error,
                    ));
                }
            }
        }
        // Win32 只发出中断请求，不保证驱动立即响应；真正结束仍依赖线程状态/join。
        Ok(())
    }
}

pub struct FsJobs {
    registry: Arc<Mutex<Registry>>,
    /// 串行化 start/shutdown，join 时不占用进度快照锁。
    lifecycle: Mutex<()>,
    worker: Mutex<Option<Worker>>,
    stopped: AtomicBool,
}

impl FsJobs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            registry: Arc::new(Mutex::new(Registry::default())),
            lifecycle: Mutex::new(()), worker: Mutex::new(None), stopped: AtomicBool::new(false),
        })
    }

    fn ensure_running(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) { Err("文件任务模块正在退出，不能开始新操作".into()) } else { Ok(()) }
    }

    pub fn list_drives(&self) -> Result<Vec<DriveInfo>, String> {
        self.ensure_running()?;
        #[cfg(windows)]
        { native::list_drives() }
        #[cfg(not(windows))]
        { Err(UNSUPPORTED.into()) }
    }

    pub fn browse(&self, path: String, data_root: Option<String>) -> Result<DirectoryView, String> {
        self.ensure_running()?;
        #[cfg(windows)]
        { native::browse(path, data_root) }
        #[cfg(not(windows))]
        { let _ = (path, data_root); Err(UNSUPPORTED.into()) }
    }

    pub fn start_copy(&self, sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<CopyJob, String> {
        #[cfg(windows)]
        { self.start_native(sources, request, data_root) }
        #[cfg(not(windows))]
        { let _ = (sources, request, data_root); Err(UNSUPPORTED.into()) }
    }

    pub fn jobs(&self) -> Result<Vec<CopyJob>, String> {
        #[cfg(not(windows))]
        { return Err(UNSUPPORTED.into()); }
        #[cfg(windows)]
        {
            let registry = self.registry.lock().map_err(|_| "读取复制任务列表失败")?;
            Ok(registry.jobs.iter().rev().cloned().collect())
        }
    }

    pub fn cancel(&self, id: String) -> Result<(), String> {
        #[cfg(not(windows))]
        { let _ = id; return Err(UNSUPPORTED.into()); }
        #[cfg(windows)]
        {
            if id.is_empty() || id.len() > 128 { return Err("复制任务 ID 无效".into()); }
            let worker = self.worker.lock().map_err(|_| "访问复制任务线程失败")?;
            if let Some(worker) = worker.as_ref().filter(|worker| worker.id == id) {
                if !worker.handle.is_finished() {
                    return worker.request_cancel();
                }
            }
            let registry = self.registry.lock().map_err(|_| "读取复制任务列表失败")?;
            if registry.jobs.iter().any(|job| job.id == id) {
                Err("该复制任务已结束，无需取消；已完成的导出不会撤销".into())
            } else {
                Err("未找到该复制任务".into())
            }
        }
    }

    /// 可重复调用；返回前活动线程已结束，不能只设置 cancel 后直接 app.exit。
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|poison| poison.into_inner());
        let worker = self.worker.lock().unwrap_or_else(|poison| poison.into_inner()).take();
        if let Some(worker) = worker {
            if let Err(error) = worker.request_cancel() {
                let mut registry = self.registry.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == worker.id && job.state == "running") {
                    // 退出流程仍等待 join；不能将中断请求失败或仍卡住的线程宣称为 cancelled。
                    job.error = Some(error);
                }
            }
            if worker.handle.join().is_err() {
                let mut registry = self.registry.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == worker.id) {
                    job.state = "failed".into();
                    job.error = Some("复制线程异常结束，未确认的文件不记为成功".into());
                }
                trim_finished(&mut registry);
            }
        }
    }

    #[cfg(windows)]
    fn start_native(&self, sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<CopyJob, String> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| "复制任务生命周期锁失败")?;
        self.ensure_running()?;
        // 没有等待队列：单个工作任务未结束时明确拒绝新请求。
        let mut slot = self.worker.lock().map_err(|_| "访问复制任务线程失败")?;
        if slot.as_ref().is_some_and(|worker| !worker.handle.is_finished()) {
            return Err("已有复制任务正在运行，请等待结束或取消后再复制".into());
        }
        if let Some(previous) = slot.take() {
            if previous.handle.join().is_err() {
                let mut registry = self.registry.lock().map_err(|_| "更新复制任务列表失败")?;
                if let Some(job) = registry.jobs.iter_mut().find(|job| job.id == previous.id) {
                    job.state = "failed".into();
                    job.error = Some("上一复制线程异常结束".into());
                }
            }
        }
        let prepared = native::prepare(sources, request, data_root)?;
        let id = native::unique_id();
        let snapshot = CopyJob {
            id: id.clone(), state: "running".into(), total: prepared.sources.len() as u64,
            processed: 0, copied: 0, skipped: 0, error: None,
        };
        {
            let mut registry = self.registry.lock().map_err(|_| "创建复制任务记录失败")?;
            trim_finished(&mut registry);
            registry.jobs.push_back(snapshot.clone());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let registry = self.registry.clone();
        let current = snapshot.clone();
        let handle = match std::thread::Builder::new().name("celesvault-export".into()).spawn(move || {
            native::run(prepared, current, worker_cancel, registry);
        }) {
            Ok(handle) => handle,
            Err(error) => {
                let mut failed = snapshot.clone();
                failed.state = "failed".into();
                failed.error = Some(crate::path_guard::io_error("启动复制线程失败", error));
                finish_snapshot(&self.registry, &failed);
                return Err(failed.error.unwrap_or_else(|| "启动复制线程失败".into()));
            }
        };
        *slot = Some(Worker { id, cancel, handle });
        // 返回的是提交时的 running 快照；是否完成以 jobs() 后续真实状态为准。
        Ok(snapshot)
    }
}

impl Drop for FsJobs {
    fn drop(&mut self) { self.shutdown(); }
}

fn trim_finished(registry: &mut Registry) {
    while registry.jobs.iter().filter(|job| job.state != "running").count() > MAX_FINISHED_JOBS {
        if let Some(index) = registry.jobs.iter().position(|job| job.state != "running") {
            registry.jobs.remove(index);
        } else { break; }
    }
}

#[cfg(windows)]
fn finish_snapshot(registry: &Arc<Mutex<Registry>>, current: &CopyJob) {
    // 只在结束路径保守恢复被 panic 污染的锁，以让 UI 看见 failed 而非永远 running。
    let mut registry = registry.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(stored) = registry.jobs.iter_mut().find(|job| job.id == current.id) {
        *stored = current.clone();
    }
    trim_finished(&mut registry);
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::path_guard::{
        ArchiveFile, DirectoryGuard, OwnedTemp, ProtectedRoot, conflict_name, file_stamp,
        io_error, local_path, safe_name, wide,
    };
    use std::{
        collections::{HashMap, HashSet},
        fs,
        io::{self, Read, Write},
        os::windows::fs::MetadataExt,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::atomic::AtomicU64,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Storage::FileSystem::{
            FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
            FILE_ATTRIBUTE_TEMPORARY, GetDriveTypeW, GetLogicalDrives,
        },
        System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
    };

    const MAX_DIRECTORY_ENTRIES: usize = 10_000;
    const MAX_COPY_SOURCES: usize = 1000;
    const MAX_NAME_ATTEMPTS: usize = 10_000;
    const MAX_TEMP_ATTEMPTS: usize = 32;
    const CHUNK_SIZE: usize = 64 * 1024;
    const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(200);
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);

    pub(super) fn unique_id() -> String {
        let ticks = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!("export-{:x}-{ticks:x}-{sequence:x}", std::process::id())
    }

    pub(super) fn list_drives() -> Result<Vec<DriveInfo>, String> {
        // SAFETY: 无指针参数的同步 Win32 调用。
        let mask = unsafe { GetLogicalDrives() };
        if mask == 0 { return Err(io_error("枚举本地驱动器失败", io::Error::last_os_error())); }
        let mut drives = Vec::new();
        for index in 0..26 {
            if mask & (1u32 << index) == 0 { continue; }
            let root = format!("{}:\\", char::from(b'A' + index as u8));
            // SAFETY: root 是本地盘符根，wide 提供 NUL 结尾且调用期间存活的缓冲。
            let kind = unsafe { GetDriveTypeW(wide(&root).as_ptr()) };
            if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE { continue; }
            // 空读卡器等无介质槽位不是已挂载卷；不能给它伪造 identity。
            match fs::symlink_metadata(&root) {
                Err(error) if matches!(error.raw_os_error(), Some(2) | Some(3) | Some(21)) => continue,
                Err(error) => return Err(io_error(&format!("读取驱动器 {root} 失败"), error)),
                Ok(_) => (),
            }
            let guard = DirectoryGuard::open(&root).map_err(|error| format!("驱动器 {root} 不可用：{error}"))?;
            let kind_label = if kind == DRIVE_REMOVABLE { "可移动磁盘" } else { "本地磁盘" };
            let label = if guard.label().is_empty() { kind_label } else { guard.label() };
            drives.push(DriveInfo {
                root: root.clone(), name: format!("{label} ({})", root.trim_end_matches('\\')),
                identity: guard.identity.clone(),
            });
        }
        Ok(drives)
    }

    pub(super) fn browse(path: String, data_root: Option<String>) -> Result<DirectoryView, String> {
        let guard = DirectoryGuard::open(&path)?;
        let protected = data_root.as_deref().map(ProtectedRoot::open).transpose()?;
        if let Some(protected) = &protected { protected.reject_target(&guard)?; }
        let read = fs::read_dir(&guard.final_path).map_err(|e| io_error("读取目录内容失败", e))?;
        let mut entries = Vec::new();
        for (index, entry) in read.enumerate() {
            // 对全部枚举项计数（包含隐藏项），避免恶意目录让一次请求无限扫描。
            if index >= MAX_DIRECTORY_ENTRIES {
                return Err("目录超过 10000 项，阶段 2 不提供不完整列表；请进入更小的子目录".into());
            }
            let entry = entry.map_err(|e| io_error("枚举目录项失败，请刷新后重试", e))?;
            let name = entry.file_name().into_string().map_err(|_| "目录包含无法安全表示的 Unicode 文件名")?;
            if name.starts_with('.') || name.starts_with("~$") { continue; }
            let real_path = entry.path();
            if protected.as_ref().is_some_and(|protected| protected.hides_entry(&real_path)) { continue; }
            let metadata = fs::symlink_metadata(&real_path).map_err(|e| io_error("读取目录项元数据失败，请刷新后重试", e))?;
            // 当前 API 没有隐藏开关/链接目标能力，默认只列普通可导航文件/目录。
            let excluded = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_TEMPORARY | FILE_ATTRIBUTE_REPARSE_POINT;
            if metadata.file_attributes() & excluded != 0 || (!metadata.is_dir() && !metadata.is_file()) { continue; }
            let display_path = guard.path.join(&name);
            local_path(display_path.to_str().ok_or("目录项路径不是有效 Unicode")?)
                .map_err(|_| "目录包含歧义或保留文件名，无法安全返回完整列表")?;
            let modified = metadata.modified().map_err(|e| io_error("读取目录项修改时间失败", e))?;
            let modified_unix_ms = modified.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64;
            entries.push(DirectoryEntry {
                name, path: display_path.to_string_lossy().into_owned(), is_dir: metadata.is_dir(),
                size: if metadata.is_file() { metadata.len() } else { 0 }, modified_unix_ms,
            });
        }
        guard.validate()?;
        if let Some(protected) = &protected { protected.reject_target(&guard)?; }
        entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));
        Ok(DirectoryView { path: guard.path.to_string_lossy().into_owned(), identity: guard.identity, entries })
    }

    #[derive(Clone, Copy)]
    enum Conflict { KeepBoth, Skip }

    struct Source {
        archive: ArchiveFile,
        name: String,
    }

    pub(super) struct Prepared {
        target: DirectoryGuard,
        protected: ProtectedRoot,
        sources: Vec<Source>,
        conflict: Conflict,
    }

    pub(super) fn prepare(sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>) -> Result<Prepared, String> {
        if sources.is_empty() { return Err("没有可导出的已归档资料".into()); }
        if sources.len() > MAX_COPY_SOURCES { return Err("单次复制最多选择 1000 条资料记录".into()); }
        let conflict = match request.conflict.as_str() {
            "keepBoth" => Conflict::KeepBoth,
            "skip" => Conflict::Skip,
            "replace" => return Err("阶段 2 禁止替换；需阶段 3 的明确冲突预览与批准后才能启用".into()),
            _ => return Err("无效的冲突策略，阶段 2 只支持保留两份或跳过".into()),
        };
        if request.target_identity.is_empty() || request.target_identity.encode_utf16().count() > 10_000 {
            return Err("缺少有效目标身份，请先浏览并重新选择目录".into());
        }
        let target = DirectoryGuard::for_copy(&request.target_path)?;
        if target.identity != request.target_identity {
            return Err("目标目录或设备身份已经变化，请刷新并重新选择，未开始复制".into());
        }
        let protected = ProtectedRoot::open(data_root.as_deref().ok_or("尚未配置持久目录，不能导出未归档资料")?)?;
        protected.reject_target(&target)?;
        let mut seen = HashSet::new();
        let mut parent_cache: HashMap<PathBuf, Arc<DirectoryGuard>> = HashMap::new();
        let mut prepared_sources = Vec::new();
        for source in sources {
            if source.id.is_empty() || source.id.len() > 128 || source.id.chars().any(char::is_control) {
                return Err("归档资料 ID 为空或超过长度限制".into());
            }
            if !seen.insert(source.id) { continue; }
            let path = protected.source_path(&source.path)?;
            let name = safe_name(&source.display_name, target.name_limit())?;
            // 在启动前检查整批文件名可以成为一个直接子项，不建立/穿入任何文件夹。
            target.child(&name)?;
            let parent_path = path.parent().ok_or("归档文件没有父目录")?.to_path_buf();
            let parents = if let Some(parents) = parent_cache.get(&parent_path) {
                parents.clone()
            } else {
                let parents = Arc::new(DirectoryGuard::open_internal(parent_path.to_str().ok_or("归档目录不是有效 Unicode")?)?);
                parent_cache.insert(parent_path, parents.clone());
                parents
            };
            // 宿主的 hash 校验租约仍存活；成功返回 start_copy 前完成所有源句柄交接。
            let archive = protected.open_archive(&path, parents)?;
            prepared_sources.push(Source { archive, name });
        }
        target.validate()?;
        protected.reject_target(&target)?;
        Ok(Prepared { target, protected, sources: prepared_sources, conflict })
    }

    enum Failure { Cancelled, Error(String) }
    impl From<String> for Failure { fn from(error: String) -> Self { Self::Error(error) } }

    fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> {
        if cancel.load(Ordering::Acquire) { Err(Failure::Cancelled) } else { Ok(()) }
    }

    fn cancelled_io(context: &str, error: io::Error, cancel: &AtomicBool) -> Failure {
        if cancel.load(Ordering::Acquire) { Failure::Cancelled } else { Failure::Error(io_error(context, error)) }
    }

    fn write_chunk(file: &mut fs::File, bytes: &[u8], cancel: &AtomicBool) -> Result<(), Failure> {
        let mut written = 0;
        while written < bytes.len() {
            check_cancel(cancel)?;
            // 不使用 write_all：它会自行重试 Interrupted，可能在取消后又发起设备 I/O。
            match file.write(&bytes[written..]) {
                Ok(0) => return Err(Failure::Error("导出暂存未能继续写入，已停止复制".into())),
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(cancelled_io("写入导出暂存失败，设备可能已拔出或空间不足", error, cancel)),
            }
        }
        Ok(())
    }

    struct Publisher { last: Instant }
    impl Publisher {
        fn publish(&mut self, registry: &Arc<Mutex<Registry>>, current: &CopyJob) -> Result<(), Failure> {
            if self.last.elapsed() < SNAPSHOT_INTERVAL { return Ok(()); }
            let mut registry = registry.lock().map_err(|_| Failure::Error("更新复制任务进度失败".into()))?;
            let stored = registry.jobs.iter_mut().find(|job| job.id == current.id)
                .ok_or_else(|| Failure::Error("复制任务记录缺失，操作已停止".into()))?;
            *stored = current.clone();
            self.last = Instant::now();
            Ok(())
        }
    }

    pub(super) fn run(mut prepared: Prepared, mut current: CopyJob, cancel: Arc<AtomicBool>, registry: Arc<Mutex<Registry>>) {
        let mut publisher = Publisher { last: Instant::now() };
        let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(), Failure> {
            for source in &mut prepared.sources {
                check_cancel(&cancel)?;
                prepared.target.validate()?;
                prepared.protected.reject_target(&prepared.target)?;
                let result = copy_one(
                    &prepared.target, &prepared.protected, prepared.conflict, source,
                    &cancel, &mut current, &mut publisher, &registry,
                );
                match result {
                    Ok(skipped) => {
                        current.processed += 1;
                        if skipped { current.skipped += 1; }
                    }
                    Err(Failure::Cancelled) => return Err(Failure::Cancelled),
                    Err(error) => { current.processed += 1; return Err(error); }
                }
                publisher.publish(&registry, &current)?;
            }
            check_cancel(&cancel)?;
            prepared.target.validate()?;
            prepared.protected.reject_target(&prepared.target)?;
            Ok(())
        }));
        match outcome {
            Ok(Ok(())) => { current.state = "completed".into(); current.error = None; }
            Ok(Err(Failure::Cancelled)) => { current.state = "cancelled".into(); current.error = None; }
            Ok(Err(Failure::Error(error))) => { current.state = "failed".into(); current.error = Some(error); }
            Err(_) => {
                current.state = "failed".into();
                current.error = Some("复制线程发生异常，未完成的资料没有记为成功".into());
            }
        }
        finish_snapshot(&registry, &current);
        // prepared 以及每个 File 在返回前析构；shutdown join 之后才退出宿主。
    }

    fn create_temp(target: &DirectoryGuard, job_id: &str) -> Result<OwnedTemp, Failure> {
        for _ in 0..MAX_TEMP_ATTEMPTS {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".celesvault-{job_id}-{sequence:x}.tmp");
            // 本任务只知道本次 create_new 的句柄；名字碰撞不删除已有文件。
            target.child(&name)?;
            match OwnedTemp::create(target, &name) {
                Ok(temp) => return Ok(temp),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("创建导出暂存失败，目标可能不可写或设备已移除", error).into()),
            }
        }
        Err(Failure::Error("暂存文件名连续冲突，未覆盖或删除任何已有文件".into()))
    }

    /// 返回 true 表示跳过。copied 在原子提交成功时累加，后置复核失败仍显示已提交数量。
    fn copy_one(
        target: &DirectoryGuard, protected: &ProtectedRoot, conflict: Conflict,
        source: &mut Source, cancel: &AtomicBool, current: &mut CopyJob,
        publisher: &mut Publisher, registry: &Arc<Mutex<Registry>>,
    ) -> Result<bool, Failure> {
        let base_name = source.name.as_str();
        if matches!(conflict, Conflict::Skip) && target.entry_exists(base_name)? {
            target.validate()?;
            return Ok(true);
        }
        check_cancel(cancel)?;
        // 仅读取 prepare 锁定的同一来源句柄；异步执行期间绝不重新按路径取源。
        let archive = &mut source.archive;
        if file_stamp(&archive.file)? != archive.stamp {
            return Err(Failure::Error("归档句柄在任务开始前变化，未开始导出".into()));
        }
        check_cancel(cancel)?;
        target.validate()?;
        let mut temp = create_temp(target, &current.id)?;
        let result = (|| -> Result<bool, Failure> {
            temp.verify()?;
            target.validate()?;
            let mut buffer = vec![0u8; CHUNK_SIZE];
            let mut copied_bytes = 0u64;
            loop {
                check_cancel(cancel)?;
                let count = match archive.file.read(&mut buffer) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(cancelled_io("读取已归档资料失败", error, cancel)),
                    Ok(count) => count,
                };
                if count == 0 { break; }
                check_cancel(cancel)?;
                write_chunk(&mut temp.file, &buffer[..count], cancel)?;
                copied_bytes = copied_bytes.checked_add(count as u64).ok_or_else(|| Failure::Error("导出字节计数溢出".into()))?;
                if copied_bytes > archive.stamp.size { return Err(Failure::Error("归档大小在复制期间变化，未提交导出".into())); }
                publisher.publish(registry, current)?;
            }
            if copied_bytes != archive.stamp.size || file_stamp(&archive.file)? != archive.stamp {
                return Err(Failure::Error("归档在读取期间发生变化，未提交导出".into()));
            }
            check_cancel(cancel)?;
            temp.file.flush().map_err(|e| cancelled_io("刷新导出暂存失败", e, cancel))?;
            temp.file.sync_all().map_err(|e| cancelled_io("同步导出暂存到磁盘失败", e, cancel))?;
            check_cancel(cancel)?;
            temp.verify()?;
            // 冲突检查只是优化；实际提交依赖 ReplaceIfExists=false 的原子句柄 rename。
            for ordinal in 0..MAX_NAME_ATTEMPTS {
                check_cancel(cancel)?;
                target.validate()?;
                protected.reject_target(target)?;
                let name = conflict_name(base_name, ordinal, target.name_limit())?;
                if target.entry_exists(&name)? {
                    if matches!(conflict, Conflict::Skip) { return Ok(true); }
                    continue;
                }
                temp.verify()?;
                match temp.publish(target, &name) {
                    Ok(()) => {
                        current.copied += 1;
                        temp.verify_published(target, &name, copied_bytes)?;
                        target.validate()?;
                        return Ok(false);
                    }
                    Err(error) => {
                        // 包括检查之后才出现的文件/文件夹：不能替换，只能跳过或另取名。
                        let name_conflict = matches!(error.raw_os_error(), Some(80) | Some(183))
                            || target.entry_exists(&name)?;
                        if name_conflict {
                            if matches!(conflict, Conflict::Skip) { return Ok(true); }
                            continue;
                        }
                        return Err(io_error("按目标目录句柄原子提交失败；此文件系统可能不支持安全重命名，未回退到路径复制", error).into());
                    }
                }
            }
            Err(Failure::Error("重名数量超过 10000，已停止且没有覆盖任何目标文件".into()))
        })();
        // 即使出错/取消也只清理此 create_new 得到的文件句柄，不删除源或外来暂存。
        if let Err(cleanup_error) = temp.cleanup() {
            let reason = match &result {
                Err(Failure::Error(error)) => format!("{error}；{cleanup_error}"),
                Err(Failure::Cancelled) => format!("复制已取消；{cleanup_error}"),
                Ok(_) => cleanup_error,
            };
            return Err(Failure::Error(reason));
        }
        result
    }
}
