use crate::{api::{ArchiveItem,CaptureSettings,EngineStatus}, archive, capture, storage::Store};
use std::{collections::HashMap,os::windows::io::AsRawHandle,path::PathBuf, sync::{Arc,Condvar,Mutex,MutexGuard,atomic::{AtomicBool,Ordering}}, thread::{self,JoinHandle},time::Duration};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_,T>,String> {
    mutex.lock().map_err(|_|"后台状态锁已损坏，操作已停止".into())
}
pub(crate) struct Inner {
    pub store: Option<Store>,
    pub paused: bool,
    pub capture_resume_after: u64,
    pub switching: bool,
    pub capture_running: bool,
    pub capture_error: Option<String>,
    pub worker_error: Option<String>,
    pub recent_error: Option<String>,
    pub health: Vec<String>,
    pub health_errors: bool,
    pub verified_archives: HashMap<String,archive::VerifiedArchive>,
    pub shutdown: bool,
}
pub(crate) struct Shared { pub inner: Mutex<Inner>, pub wake: Condvar, pub wait: Mutex<()> }
impl Shared {
    pub fn capture_error(&self,error: String) { if let Ok(mut i)=lock(&self.inner) {i.capture_running=false;i.capture_error=Some(bounded(error));} }
    pub fn capture_started(&self) {if let Ok(mut i)=lock(&self.inner) {i.capture_running=true;i.capture_error=None;} }
    pub fn note_capture_error(&self,error:String) {if let Ok(mut i)=lock(&self.inner) {i.recent_error=Some(bounded(error));} }
    pub fn capture_health(&self,c: capture::probe::model::Counters,h:capture::probe::model::TraceHealth,unknown:u64,identity:u64) {
        if let Ok(mut i)=lock(&self.inner) {
            // 正常路径/进程未知统计保持可见但不等同故障；实际丢失、解析失败、
            // 回调/会话失败、已过滤 WPS 候选的身份拒绝才提升为错误。
            i.health_errors=c.queue_dropped>0 || c.schema_failures>0 || c.property_failures>0 || c.callback_panics>0
                || h.events_lost>0 || h.realtime_buffers_lost>0 || h.query_code!=0 || h.stop_code!=0
                || ![0,windows_sys::Win32::Foundation::ERROR_CTX_CLOSE_PENDING].contains(&h.close_code)
                || h.consumer_code.is_some_and(|code|![0,windows_sys::Win32::Foundation::ERROR_CANCELLED].contains(&code)) || identity>0;
            i.health=vec![
                "识别依据为文件 I/O 与实际进程路径；不能完美区分预览/人工打开".into(),
                format!("ETW：事件 {}，范围内 {}，队列丢弃 {}，名称缓存重置 {}",c.callback_events,c.scope_matching_events,c.queue_dropped,c.name_cache_resets),
                format!("未解析路径 {}，未归属进程 {}，候选未知进程路径 {}，源身份拒绝 {}",c.path_unresolved_events,c.unknown_attribution_matching_events,unknown,identity),
                format!("Schema 失败 {}，属性失败 {}，回调异常 {}",c.schema_failures,c.property_failures,c.callback_panics),
                format!("ETW 丢失事件 {}，实时缓冲丢失 {}，查询码 {}，停止码 {}，关闭码 {}，消费码 {:?}，已 join {}",h.events_lost,h.realtime_buffers_lost,h.query_code,h.stop_code,h.close_code,h.consumer_code,h.consumer_joined),
            ];
        }
    }
}
fn bounded(text:String)->String {text.chars().take(1000).collect()}
fn cache_verified(inner:&mut Inner,id:String,verified:archive::VerifiedArchive) {
    const LIMIT:usize=2048;
    if inner.verified_archives.len()>=LIMIT && !inner.verified_archives.contains_key(&id) {inner.verified_archives.clear();}
    inner.verified_archives.insert(id,verified);
}
struct Service { stop:Arc<AtomicBool>,capture:Option<JoinHandle<()>>,worker:Option<JoinHandle<()>> }

/// 与 Tauri/async 无关；同步方法可在宿主 spawn_blocking 中调用。
/// worker 只持有 Shared，避免 Arc<VaultEngine> 自循环；退出显式 stop + join。
pub struct VaultEngine {shared:Arc<Shared>,service:Mutex<Option<Service>>,lifecycle:Mutex<()>}
impl VaultEngine {
    pub fn new()->Arc<Self> {
        Arc::new(Self {shared:Arc::new(Shared {inner:Mutex::new(Inner {store:None,paused:false,capture_resume_after:0,switching:false,capture_running:false,capture_error:None,worker_error:None,recent_error:None,health:Vec::new(),health_errors:false,verified_archives:HashMap::new(),shutdown:false}),wake:Condvar::new(),wait:Mutex::new(())}),service:Mutex::new(None),lifecycle:Mutex::new(())})
    }
    pub fn status(&self)->Result<EngineStatus,String> {
        let inner=lock(&self.shared.inner)?;
        let mut root_error=None;
        let (pending,failed)=match &inner.store {
            Some(s)=>match s.counts() {Ok(counts)=>counts,Err(e)=>{root_error=Some(e);(0,0)}},None=>(0,0)
        };
        let data_root=inner.store.as_ref().map(|s|match s.display_root() {
            Ok(path)=>path.to_string_lossy().into_owned(),
            Err(e)=>{root_error=Some(e);s.configured_root.to_string_lossy().into_owned()}
        });
        let state=if inner.store.is_none() {"unconfigured"} else if root_error.is_some() || inner.shutdown || inner.capture_error.is_some() || inner.worker_error.is_some() {"error"}
            else if inner.paused {"paused"} else if inner.capture_running {"running"} else {"error"};
        let detail=match state {
            "unconfigured"=>"尚未选择持久目录，未启动真实监测".into(),
            "paused"=>"监测已暂停；已接受归档任务仍会完成".into(),
            "running"=>format!("真实 Windows ETW 监测运行中，待归档 {pending}，失败 {failed}；当前使用每日未分类记录"),
            _=>root_error.clone().or(inner.capture_error.clone()).or(inner.worker_error.clone()).unwrap_or_else(||if inner.shutdown {"引擎已退出".into()}else{"监测正在启动，尚不能确认运行".into()}),
        };
        let mut health=inner.health.clone();
        if let Some(error)=&root_error {health.push(format!("原数据卷错误：{error}"));}
        if let Some(error)=&inner.recent_error {health.push(format!("最近候选错误：{error}"));}
        if let Some(error)=&inner.worker_error {health.push(format!("归档后台错误：{error}"));}
        let has_errors=root_error.is_some() || failed>0 || inner.health_errors || inner.capture_error.is_some() || inner.worker_error.is_some() || inner.recent_error.is_some();
        Ok(EngineStatus {data_root,monitor_state:state.into(),monitor_detail:detail,pending_count:pending,failed_count:failed,has_errors,health})
    }
    pub fn configure_root(&self,path:String)->Result<EngineStatus,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        {
            let mut inner=lock(&self.shared.inner)?;
            if inner.shutdown {return Err("引擎已经关闭".into());}
            if let Some(store)=&inner.store {
                if let Ok(current)=store.display_root() && capture::probe::filter::normalize(&current.to_string_lossy()).eq_ignore_ascii_case(&capture::probe::filter::normalize(&path)) {drop(inner);return self.status();}
                if store.counts()?.0>0 {return Err("仍有活动/待重试捕获任务，不能更换持久目录；不会静默迁移数据库".into());}
                let old=store.display_root()?;
                if capture::probe::filter::under(&path,&old.to_string_lossy()) || capture::probe::filter::under(&old.to_string_lossy(),&path) {
                    return Err("新旧数据目录不能互为父子目录；不会在旧归档内部建立另一个数据库".into());
                }
            }
            inner.switching=true;
        }
        self.stop_service();
        match Store::open(path) {
            Ok(store)=>{
                let mut inner=lock(&self.shared.inner)?;
                inner.store=Some(store);inner.switching=false;inner.capture_running=false;inner.capture_error=None;inner.worker_error=None;inner.recent_error=None;inner.health.clear();inner.health_errors=false;inner.verified_archives.clear();
            }
            Err(error)=>{
                lock(&self.shared.inner)?.switching=false;
                if lock(&self.shared.inner)?.store.is_some() {let _=self.start_service();}
                return Err(error);
            }
        }
        self.start_service()?;
        self.status()
    }
    pub fn list_today(&self)->Result<Vec<ArchiveItem>,String> {
        let (root,items,entities)={
            let inner=lock(&self.shared.inner)?;
            let Some(store)=&inner.store else{return Ok(Vec::new());};
            let items=store.today()?;
            let entities=items.iter().filter(|item|item.state=="available").map(|item| {
                Ok((store.entity(&item.id)?,inner.verified_archives.get(&item.id).cloned()))
            }).collect::<Result<Vec<_>,String>>()?;
            (store.root.clone(),items,entities)
        };
        let mut invalid=Vec::new();let mut validated=Vec::new();
        for (entity,cached) in entities {
            match archive::check_cached(&root,&entity,cached.as_ref()) {
                Ok(fingerprint)=>validated.push((entity.id.clone(),archive::VerifiedArchive::new(&entity,fingerprint))),
                Err(error)=>invalid.push((entity.id,error)),
            }
        }
        let mut inner=lock(&self.shared.inner)?;
        if inner.store.as_ref().is_none_or(|s|s.root!=root) {return Err("数据目录在查询期间更换，请重试".into());}
        for (id,verified) in validated {cache_verified(&mut inner,id,verified);}
        if invalid.is_empty() {return Ok(items);}
        for (id,error) in invalid {
            inner.verified_archives.remove(&id);
            inner.store.as_mut().ok_or("数据目录尚未配置")?.invalidate(&id,&error)?;
        }
        inner.store.as_ref().ok_or("数据目录尚未配置")?.today()
    }
    pub fn pause(&self,paused:bool)->Result<EngineStatus,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        {let mut i=lock(&self.shared.inner)?;if i.shutdown{return Err("引擎已关闭".into());}i.paused=paused;i.capture_resume_after=crate::storage::now_ms();}
        // 消费线程保持活动以维护路径/PID关联与健康；暂停期间不接受新捕获意图。
        self.status()
    }
    pub fn retry(&self,task_id:String)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let inner=lock(&self.shared.inner)?;
        if inner.shutdown || inner.switching {return Err("引擎不可接受重试".into());}
        inner.store.as_ref().ok_or("数据目录尚未配置")?.retry(&task_id)?;
        self.shared.wake.notify_all();
        Ok(())
    }
    pub fn archived_path(&self,id:String)->Result<PathBuf,String> {
        self.lease_archive(id)?.validated_display_path()
    }
    pub fn lease_archive(&self,id:String)->Result<archive::ArchiveLease,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let (root,entity)={
            let inner=lock(&self.shared.inner)?;
            let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;
            (store.root.clone(),store.entity(&id)?)
        };
        match archive::lease(&root,&entity,None) {
            Ok(lease)=>{
                let mut inner=lock(&self.shared.inner)?;
                if let Some(store)=&inner.store && store.root==root {
                    cache_verified(&mut inner,id,archive::VerifiedArchive::new(&entity,lease.fingerprint.clone()));
                }
                Ok(lease)
            }
            Err(error)=>{
                let mut inner=lock(&self.shared.inner)?;
                inner.verified_archives.remove(&id);
                if let Some(store)=&mut inner.store && store.root==root {store.invalidate(&id,&error)?;}
                Err(error)
            }
        }
    }
    pub fn get_capture_settings(&self)->Result<CaptureSettings,String> {
        let inner=lock(&self.shared.inner)?;
        let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;
        store.validate()?;
        Ok(store.settings.clone())
    }
    pub fn set_capture_settings(&self,settings:CaptureSettings)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let settings=capture::validate_settings(settings)?;
        let mut inner=lock(&self.shared.inner)?;
        if inner.shutdown || inner.switching {return Err("引擎不可更新设置".into());}
        inner.store.as_mut().ok_or("数据目录尚未配置")?.set_settings(settings)?;
        Ok(())
    }
    pub fn source_defaults(&self)->Result<Vec<crate::api::SourceRule>,String> {Ok(capture::source_defaults())}
    pub fn shutdown(&self) {
        let Ok(_lifecycle)=lock(&self.lifecycle) else{return;};
        if let Ok(mut i)=lock(&self.shared.inner) {i.shutdown=true;i.switching=true;i.capture_running=false;}
        self.stop_service();
        // 保留 root/status 与已提交意图。SQLite 每次 FULL 提交，无异步尾写。
    }
    fn start_service(&self)->Result<(),String> {
        let stop=Arc::new(AtomicBool::new(false));
        let worker_shared=self.shared.clone();let worker_stop=stop.clone();
        let worker=thread::Builder::new().name("celesvault-archive".into()).spawn(move|| {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||worker_loop(worker_shared.clone(),worker_stop)));
            if result.is_err() && let Ok(mut i)=lock(&worker_shared.inner) {i.worker_error=Some("归档后台异常中止；保留意图等待重启恢复".into());}
        }).map_err(|e|format!("无法启动归档后台：{e}"))?;
        let shared=self.shared.clone();let capture_stop=stop.clone();
        let capture=thread::Builder::new().name("celesvault-capture".into()).spawn(move|| {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::run(shared.clone(),capture_stop))).is_err() {
                shared.capture_error("监测线程异常中止，未模拟成功".into());
            }
        });
        match capture {
            Ok(capture)=>{*lock(&self.service)?=Some(Service {stop,capture:Some(capture),worker:Some(worker)});Ok(())}
            Err(error)=>{stop.store(true,Ordering::Release);self.shared.wake.notify_all();let _=worker.join();self.shared.capture_error(error.to_string());Err(format!("无法启动监测后台：{error}"))}
        }
    }
    fn stop_service(&self) {
        let service=self.service.lock().unwrap_or_else(|p|p.into_inner()).take();
        if let Some(mut service)=service {
            service.stop.store(true,Ordering::Release);self.shared.wake.notify_all();
            // 停止标志中断下一块/重试；请求取消线程当前的同步设备 I/O，避免拔盘读卡住。
            // handle 由 JoinHandle 保有且仍存活；ERROR_NOT_FOUND（当前无 I/O）不是故障。
            for thread in [service.capture.as_ref(),service.worker.as_ref()].into_iter().flatten() {
                unsafe {windows_sys::Win32::System::IO::CancelSynchronousIo(thread.as_raw_handle().cast());}
            }
            if let Some(capture)=service.capture.take() && capture.join().is_err() {self.shared.capture_error("监测线程 join 失败".into());}
            if let Some(worker)=service.worker.take() && worker.join().is_err() {
                if let Ok(mut i)=lock(&self.shared.inner) {i.worker_error=Some("归档线程 join 失败".into());}
            }
        }
    }
}
impl Drop for VaultEngine {fn drop(&mut self) {self.shutdown();}}

fn worker_loop(shared:Arc<Shared>,stop:Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let task=(|| {
            let mut inner=lock(&shared.inner)?;
            if inner.switching {return Ok(None);}
            inner.store.as_mut().ok_or("数据目录尚未配置")?.due()
        })();
        match task {
            Ok(Some(task))=>{
                let result=archive::execute(&shared,task.clone(),&stop);
                if let Err(error)=result {
                    if let Ok(inner)=lock(&shared.inner) && let Some(store)=&inner.store {
                        if let Err(e)=store.fail(&task,&error.message,error.terminal) {drop(inner);if let Ok(mut i)=lock(&shared.inner) {i.worker_error=Some(bounded(e));}}
                    }
                }
            }
            Ok(None)=>{
                let guard=shared.wait.lock().unwrap_or_else(|p|p.into_inner());
                let _wait=shared.wake.wait_timeout(guard,Duration::from_millis(250));
            }
            Err(error)=>{
                if let Ok(mut i)=lock(&shared.inner) {i.worker_error=Some(bounded(error));}
                break;
            }
        }
    }
}
