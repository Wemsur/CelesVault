use crate::{api::*, activity::ArchiveActivity, archive, capture, recovery, secure_fs, storage::{self, Store}, thumbnail};
use std::{collections::{HashMap,HashSet},os::windows::io::AsRawHandle,path::PathBuf,sync::{Arc,Condvar,Mutex,MutexGuard,atomic::{AtomicBool,Ordering},mpsc},thread::{self,JoinHandle},time::{Duration,Instant}};

pub(crate) fn lock<T>(mutex:&Mutex<T>) -> Result<MutexGuard<'_,T>,String> {mutex.lock().map_err(|_|"后台状态锁已损坏，操作已停止".into())}
pub(crate) struct Inner {
    pub store:Option<Store>, pub paused:bool, pub capture_resume_after:u64, pub capture_event_after:u64, pub switching:bool,
    pub capture_running:bool, pub capture_error:Option<String>, pub worker_error:Option<String>,
    pub recent_error:Option<String>, pub health:Vec<String>, pub health_errors:bool,
    pub verified_archives:HashMap<String,archive::VerifiedArchive>, pub shutdown:bool,
    pub capture_activity:Option<CaptureActivity>,
}
impl Inner {
    // 与 accept/完成信号共用 inner 锁排序；即使两次宿主轮询间已完成，也保留最新事件。
    pub(crate) fn record_capture_activity(&mut self,kind:CaptureActivityKind) {
        let sequence=self.capture_activity.map_or(1,|activity|activity.sequence.saturating_add(1));
        self.capture_activity=Some(CaptureActivity {sequence,kind,occurred_at:Instant::now()});
    }
}
pub(crate) struct Shared {
    pub inner:Mutex<Inner>, pub wake:Condvar, pub wait:Mutex<()>, pub activity:Arc<ArchiveActivity>, pub course_admission:Mutex<()>,
}
impl Shared {
    pub fn capture_error(&self,error:String) {if let Ok(mut i)=lock(&self.inner) {i.capture_running=false;i.capture_error=Some(bounded(error));}}
    pub fn capture_started(&self) {if let Ok(mut i)=lock(&self.inner) {i.capture_running=true;i.capture_error=None;}}
    pub fn note_capture_error(&self,error:String) {if let Ok(mut i)=lock(&self.inner) {i.recent_error=Some(bounded(error));}}
    pub fn capture_health(&self,c:capture::probe::model::Counters,h:capture::probe::model::TraceHealth,unknown:u64,identity:u64) {
        if let Ok(mut i)=lock(&self.inner) {
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
    if inner.verified_archives.len()>=2048 && !inner.verified_archives.contains_key(&id) {inner.verified_archives.clear();}
    inner.verified_archives.insert(id,verified);
}
fn ready(inner:&Inner)->Result<(),String> {
    if inner.shutdown || inner.switching {return Err("引擎处于关闭/换根状态，操作已拒绝".into());}
    inner.store.as_ref().ok_or("数据目录尚未配置；不会自动选择或迁移持久根")?.validate()
}
struct Service {stop:Arc<AtomicBool>,capture:Option<JoinHandle<Result<(),String>>>,worker:Option<JoinHandle<()>>,stop_error:Option<String>}

/// 与 Tauri/async 无关。同步 Result 方法可由宿主 spawn_blocking 调用。
/// 生命周期锁只由最外层入口获取；内部 helper 不递归调用带锁的 public method。
pub struct VaultEngine {shared:Arc<Shared>,service:Mutex<Option<Service>>,lifecycle:Mutex<()>}
impl VaultEngine {
    pub fn new()->Arc<Self> {
        Arc::new(Self {shared:Arc::new(Shared {inner:Mutex::new(Inner {store:None,paused:false,capture_resume_after:0,capture_event_after:0,switching:false,
            capture_running:false,capture_error:None,worker_error:None,recent_error:None,health:Vec::new(),health_errors:false,
            verified_archives:HashMap::new(),shutdown:false,capture_activity:None}),wake:Condvar::new(),wait:Mutex::new(()),activity:Arc::new(ArchiveActivity::default()),course_admission:Mutex::new(())}),service:Mutex::new(None),lifecycle:Mutex::new(())})
    }
    /// 托盘专用运行时快照，不改变跨平台 EngineStatus/前端协议。
    pub fn capture_activity(&self)->Result<Option<CaptureActivity>,String> {Ok(lock(&self.shared.inner)?.capture_activity)}
    pub fn status(&self)->Result<EngineStatus,String> {
        let inner=lock(&self.shared.inner)?;let mut root_error=None;let mut recovery_errors=Vec::new();
        let (pending,failed)=match &inner.store {
            Some(store)=>{match store.recovery_errors() {Ok(errors)=>recovery_errors=errors,Err(error)=>root_error=Some(error)};
                match store.counts() {Ok(counts)=>counts,Err(error)=>{root_error=Some(error);(0,0)}}},None=>(0,0),
        };
        let data_root=inner.store.as_ref().map(|store|match store.display_root() {
            Ok(path)=>path.to_string_lossy().into_owned(),Err(error)=>{root_error=Some(error);store.configured_root.to_string_lossy().into_owned()},
        });
        let state=if inner.store.is_none() {"unconfigured"} else if root_error.is_some() || inner.shutdown || inner.switching || inner.capture_error.is_some() || inner.worker_error.is_some() {"error"}
            else if !inner.capture_running {"error"} else if inner.paused {"paused"} else {"running"};
        let detail=match state {
            "unconfigured"=>"尚未选择持久目录，未启动真实监测".into(),
            "paused"=>"监测已暂停；允许的已接受归档任务仍会完成".into(),
            "running"=>format!("真实 Windows ETW 监测运行中，待归档 {pending}，失败 {failed}；归属按首次时间与当日课程快照"),
            _=>root_error.clone().or(inner.capture_error.clone()).or(inner.worker_error.clone()).unwrap_or_else(||if inner.shutdown {"引擎已退出".into()}else if inner.switching {"正在应用监测设置/停止或启动后台，暂不接受捕获".into()}else {"监测正在启动，尚不能确认运行".into()}),
        };
        let mut health=inner.health.clone();
        if let Some(error)=&root_error {health.push(format!("原数据卷错误：{error}"));}
        if let Some(error)=&inner.recent_error {health.push(format!("最近操作/候选错误：{error}"));}
        if let Some(error)=&inner.worker_error {health.push(format!("归档后台错误：{error}"));}
        health.extend(recovery_errors.iter().map(|e|format!("回收恢复错误：{e}")));
        let has_errors=root_error.is_some() || failed>0 || inner.health_errors || !recovery_errors.is_empty() || inner.capture_error.is_some() || inner.worker_error.is_some() || inner.recent_error.is_some();
        Ok(EngineStatus {data_root,monitor_state:state.into(),monitor_detail:detail,pending_count:pending,failed_count:failed,has_errors,health})
    }
    pub fn configure_root(&self,path:String)->Result<EngineStatus,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        {
            let mut inner=lock(&self.shared.inner)?;
            if inner.shutdown {return Err("引擎已经关闭".into());}
            if let Some(store)=&inner.store {
                if let Ok(current)=store.display_root() && capture::probe::filter::normalize(&current.to_string_lossy()).eq_ignore_ascii_case(&capture::probe::filter::normalize(&path)) {
                    let restart=inner.switching || inner.capture_error.is_some() || inner.worker_error.is_some();drop(inner);
                    if restart {self.restart_locked()?;}
                    return self.status();
                }
                if store.counts()?.0>0 || self.shared.activity.busy()? {return Err("仍有捕获任务/读写租约，不能更换持久目录；不会迁移数据库".into());}
                if !store.intents()?.is_empty() {return Err("仍有未完成回收意图，不能更换持久目录".into());}
                let old=store.display_root()?;
                if capture::probe::filter::under(&path,&old.to_string_lossy()) || capture::probe::filter::under(&old.to_string_lossy(),&path) {
                    return Err("新旧数据目录不能互为父子目录；不会在旧归档内部建立新库".into());
                }
            }
            inner.switching=true;
        }
        if let Err(error)=self.stop_service() {self.transition_error(&error);return Err(error);}
        match Store::open(path) {
            Ok(store)=>{
                let mut inner=lock(&self.shared.inner)?;
                inner.store=Some(store);inner.switching=false;inner.capture_running=false;inner.capture_error=None;inner.worker_error=None;inner.recent_error=None;
                inner.health.clear();inner.health_errors=false;inner.verified_archives.clear();
            }
            Err(error)=>{
                let has_old={let mut inner=lock(&self.shared.inner)?;inner.switching=false;inner.store.is_some()};
                if has_old {if let Err(restart)=self.resume_service() {return Err(format!("{error}；恢复原目录后台也失败：{restart}"));}}
                return Err(error);
            }
        }
        self.resume_service()?;self.status()
    }
    pub fn config(&self)->Result<ConfigBundle,String> {let inner=lock(&self.shared.inner)?;ready(&inner)?;inner.store.as_ref().ok_or("数据目录尚未配置")?.config()}
    pub fn save_config(&self,bundle:ConfigBundle)->Result<ConfigBundle,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        self.save_config_locked(crate::config::validate_config(bundle)?)
    }
    fn save_config_locked(&self,bundle:ConfigBundle)->Result<ConfigBundle,String> {
        let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        let capture_changed=inner.store.as_ref().is_some_and(|s|s.settings!=bundle.capture);
        let exclusion_changed=inner.store.as_ref().is_some_and(|s|s.settings.excluded_subject_ids!=bundle.capture.excluded_subject_ids);
        if !exclusion_changed {
            let saved=inner.store.as_mut().ok_or("数据目录尚未配置")?.save_config(bundle)?;
            if capture_changed {
                inner.capture_resume_after=storage::now_ms();
                inner.capture_event_after=inner.capture_event_after.max(inner.capture_resume_after);
            }
            self.shared.wake.notify_all();return Ok(saved);
        }
        // stop–commit–resume：等待期间不持有 worker 需要的 inner 锁。
        inner.switching=true;inner.capture_running=false;drop(inner);
        if let Err(error)=self.stop_service() {self.transition_error(&error);return Err(error);}
        let saved=(|| {
            let mut inner=lock(&self.shared.inner)?;
            let store=inner.store.as_mut().ok_or("数据目录尚未配置")?;
            store.validate()?;
            store.recover_and_save_config(bundle)
        })();
        match saved {
            Ok(saved)=>{
                self.resume_service().map_err(|error|format!("配置已持久保存，但恢复监测失败，服务保持停止：{error}"))?;
                Ok(saved)
            },
            Err(error)=>{self.transition_error(&error);Err(error)},
        }
    }
    /// 独立生命周期入口；保留持久根、历史、配置与暂停标记。
    pub fn restart_monitoring(&self)->Result<EngineStatus,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        self.restart_locked()?;
        self.status()
    }
    fn restart_locked(&self)->Result<(),String> {
        {
            let mut inner=lock(&self.shared.inner)?;
            if inner.shutdown {return Err("引擎已经关闭".into());}
            inner.store.as_ref().ok_or("数据目录尚未配置")?.validate()?;
            inner.switching=true;inner.capture_running=false;
        }
        if let Err(error)=self.stop_service() {self.transition_error(&error);return Err(error);}
        let recovered=(|| {let inner=lock(&self.shared.inner)?;inner.store.as_ref().ok_or("数据目录尚未配置")?.recover_copying()})();
        if let Err(error)=recovered {self.transition_error(&error);return Err(error);}
        self.resume_service()
    }
    fn transition_error(&self,error:&str) {
        if let Ok(mut inner)=lock(&self.shared.inner) {
            inner.switching=true;inner.capture_running=false;inner.worker_error=Some(bounded(error.into()));
        }
    }
    fn resume_service(&self)->Result<(),String> {
        {
            let mut inner=lock(&self.shared.inner)?;
            inner.capture_resume_after=storage::now_ms();
            inner.capture_event_after=inner.capture_event_after.max(inner.capture_resume_after);
            inner.capture_running=false;
            inner.capture_error=None;inner.worker_error=None;inner.switching=false;
        }
        if let Err(error)=self.start_service() {self.transition_error(&error);return Err(error);}
        Ok(())
    }
    pub fn import_config(&self,text:String)->Result<ConfigBundle,String> {crate::config::parse_config(&text)}
    pub fn export_config(&self)->Result<String,String> {serde_json::to_string_pretty(&self.config()?).map_err(|e|format!("导出配置失败：{e}"))}
    pub fn import_timetable(&self,text:String,format:String)->Result<Timetable,String> {crate::timetable::import_timetable(&text,&format)}
    pub fn export_timetable(&self,format:String)->Result<String,String> {crate::timetable::export_timetable(&self.config()?.timetable,&format)}
    pub fn courses(&self,date:String)->Result<Vec<CourseView>,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let courses={let mut inner=lock(&self.shared.inner)?;ready(&inner)?;inner.store.as_mut().ok_or("数据目录尚未配置")?.courses(&date)?};
        let items:Vec<_>=courses.iter().flat_map(|c|c.items.iter().cloned()).collect();
        self.validate_available(&items)?;
        lock(&self.shared.inner)?.store.as_mut().ok_or("数据目录尚未配置")?.courses(&date)
    }
    pub fn list_today(&self)->Result<Vec<ArchiveItem>,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let items={let mut inner=lock(&self.shared.inner)?;if inner.store.is_none() {return Ok(Vec::new());}ready(&inner)?;inner.store.as_mut().ok_or("数据目录尚未配置")?.today()?};
        self.validate_available(&items)?;
        lock(&self.shared.inner)?.store.as_mut().ok_or("数据目录尚未配置")?.today()
    }
    fn validate_available(&self,items:&[ArchiveItem])->Result<(),String> {
        let (root,entities)={
            let inner=lock(&self.shared.inner)?;let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;store.validate()?;
            let mut ids=HashSet::new();let entities=items.iter().filter(|item|item.state=="available" && ids.insert(item.id.clone())).map(|item|
                Ok((store.entity(&item.id)?,inner.verified_archives.get(&item.id).cloned()))).collect::<Result<Vec<_>,String>>()?;
            (store.root.clone(),entities)
        };
        let mut validated=Vec::new();let mut invalid=Vec::new();let mut busy=None;
        for (entity,cached) in entities {
            if entity.state!="available" {continue;}
            let _read=match self.shared.activity.read(&entity.id) {Ok(token)=>token,Err(error)=>{busy=Some(error);continue;}};
            match archive::check_cached(&root,&entity,cached.as_ref()) {
                Ok(fingerprint)=>validated.push((entity, fingerprint)),
                Err(error) if secure_fs::busy_error(&error)=>busy=Some(error),
                Err(error)=>invalid.push((entity.id,error)),
            }
        }
        let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        if inner.store.as_ref().is_none_or(|s|s.root!=root) {return Err("数据目录在查询期间更换，请重试".into());}
        for (entity,fingerprint) in validated {
            inner.store.as_ref().ok_or("数据目录尚未配置")?.pin_entity_identity(&entity,&fingerprint.identity)?;
            cache_verified(&mut inner,entity.id.clone(),archive::VerifiedArchive::new(&entity,fingerprint));
        }
        for (id,error) in invalid {inner.verified_archives.remove(&id);inner.store.as_mut().ok_or("数据目录尚未配置")?.invalidate(&id,&error)?;}
        if let Some(error)=busy {return Err(error);}Ok(())
    }
    pub fn change_course(&self,course_id:String,subject_id:String)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let _admission=lock(&self.shared.course_admission)?;
        let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.change_course(&course_id,&subject_id)
    }
    pub fn library(&self,query:LibraryQuery)->Result<LibraryPage,String> {
        let _lifecycle=lock(&self.lifecycle)?;let inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_ref().ok_or("数据目录尚未配置")?.library(query)
    }
    pub fn detail(&self,id:String)->Result<ArchiveDetail,String> {
        let _lifecycle=lock(&self.lifecycle)?;let inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_ref().ok_or("数据目录尚未配置")?.detail(&id)
    }
    pub fn edit_archive(&self,id:String,name:String,course_id:Option<String>)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let _write=self.shared.activity.write(&id)?;
        let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.edit_archive(&id,&name,course_id.as_deref())
    }
    pub fn move_usage(&self,id:String,from_course_id:String,to_course_id:String)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let _write=self.shared.activity.write(&id)?;
        let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.move_usage(&id,&from_course_id,&to_course_id)
    }
    pub fn favorite_save(&self,folder:FixedFolder)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.favorite_save(folder)
    }
    pub fn favorite_remove(&self,id:String)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_mut().ok_or("数据目录尚未配置")?.favorite_remove(&id)
    }
    pub fn trash(&self,id:String)->Result<(),String> {self.file_action(id,"trash")}
    pub fn restore(&self,id:String)->Result<(),String> {self.file_action(id,"restore")}
    pub fn purge(&self,id:String)->Result<(),String> {self.file_action(id,"purge")}
    fn file_action(&self,id:String,kind:&str)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let _write=self.shared.activity.write(&id)?;
        let result=recovery::action(&self.shared,&id,kind);
        let prefix=format!("回收资料 {id}：");
        match &result {
            Err(error)=>self.shared.note_capture_error(format!("{prefix}{error}")),
            Ok(())=>{let mut inner=lock(&self.shared.inner)?;if inner.recent_error.as_ref().is_some_and(|e|e.starts_with(&prefix)) {inner.recent_error=None;}}
        }
        result
    }
    pub fn trash_list(&self)->Result<Vec<ArchiveDetail>,String> {
        let _lifecycle=lock(&self.lifecycle)?;let inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_ref().ok_or("数据目录尚未配置")?.trash_list()
    }
    pub fn busy_volumes(&self)->Result<Vec<String>,String> {
        let inner=lock(&self.shared.inner)?;
        let mut volumes=match &inner.store {Some(store)=>store.busy_volumes()?,None=>Vec::new()};drop(inner);
        volumes.extend(self.shared.activity.source_volumes()?);volumes.sort();volumes.dedup();Ok(volumes)
    }
    pub fn thumbnail(&self,id:String)->Result<Option<String>,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        let (root,entity)={let inner=lock(&self.shared.inner)?;ready(&inner)?;let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;(store.root.clone(),store.entity(&id)?)};
        if entity.state!="available" {return Err("回收/失效资料不可生成或读取缩略图".into());}
        if thumbnail::supported(&entity.extension).is_none() {return Ok(None);}
        if entity.size>thumbnail::MAX_SOURCE_BYTES {return Err("图片超过 64 MiB 缩略图输入上限；归档仍可正常校验/导出".into());}
        let lease=self.lease_archive_locked(&id)?;
        let result=thumbnail::get(&self.shared,&root,&entity,&lease);
        if let Err(error)=&result {self.shared.note_capture_error(error.clone());}result
    }
    pub fn pause(&self,paused:bool)->Result<EngineStatus,String> {
        let _lifecycle=lock(&self.lifecycle)?;
        {let mut inner=lock(&self.shared.inner)?;if inner.shutdown {return Err("引擎已关闭".into());}inner.paused=paused;inner.capture_resume_after=storage::now_ms();inner.capture_event_after=inner.capture_event_after.max(inner.capture_resume_after);}
        self.status()
    }
    pub fn retry(&self,task_id:String)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let inner=lock(&self.shared.inner)?;ready(&inner)?;
        inner.store.as_ref().ok_or("数据目录尚未配置")?.retry(&task_id)?;self.shared.wake.notify_all();Ok(())
    }
    /// 兼容阶段2；新宿主系统打开应持有 lease_archive 并使用 lease.display_path。
    pub fn archived_path(&self,id:String)->Result<PathBuf,String> {self.lease_archive(id)?.display_path()}
    pub fn lease_archive(&self,id:String)->Result<archive::ArchiveLease,String> {let _lifecycle=lock(&self.lifecycle)?;self.lease_archive_locked(&id)}
    fn lease_archive_locked(&self,id:&str)->Result<archive::ArchiveLease,String> {
        let (root,entity)={let inner=lock(&self.shared.inner)?;ready(&inner)?;let store=inner.store.as_ref().ok_or("数据目录尚未配置")?;(store.root.clone(),store.entity(id)?)};
        if entity.state!="available" {return Err(format!("归档资料不可用（{}）；不能租约/打开已回收或失效资料",entity.state));}
        let token=self.shared.activity.read(id)?;
        match archive::lease(&root,&entity,None) {
            Ok(mut lease)=>{
                let mut inner=lock(&self.shared.inner)?;ready(&inner)?;
                inner.store.as_ref().ok_or("数据目录尚未配置")?.pin_entity_identity(&entity,&lease.fingerprint.identity)?;
                cache_verified(&mut inner,id.into(),archive::VerifiedArchive::new(&entity,lease.fingerprint.clone()));
                lease.activity=Some(token);Ok(lease)
            }
            Err(error)=>{
                if !secure_fs::busy_error(&error) {let mut inner=lock(&self.shared.inner)?;inner.verified_archives.remove(id);inner.store.as_mut().ok_or("数据目录尚未配置")?.invalidate(id,&error)?;}
                Err(error)
            }
        }
    }
    pub fn get_capture_settings(&self)->Result<CaptureSettings,String> {Ok(self.config()?.capture)}
    pub fn set_capture_settings(&self,settings:CaptureSettings)->Result<(),String> {
        let _lifecycle=lock(&self.lifecycle)?;let settings=capture::validate_settings(settings)?;
        let mut bundle={let inner=lock(&self.shared.inner)?;ready(&inner)?;inner.store.as_ref().ok_or("数据目录尚未配置")?.config()?};
        bundle.capture=settings;
        self.save_config_locked(bundle).map(|_|())
    }
    pub fn source_defaults(&self)->Result<Vec<SourceRule>,String> {Ok(capture::source_defaults())}
    pub fn shutdown(&self) {
        let Ok(_lifecycle)=lock(&self.lifecycle) else {return;};
        if let Ok(mut inner)=lock(&self.shared.inner) {inner.shutdown=true;inner.switching=true;inner.capture_running=false;}
        if let Err(error)=self.stop_service() {if let Ok(mut inner)=lock(&self.shared.inner) {inner.worker_error=Some(bounded(error));}}
    }
    fn start_service(&self)->Result<(),String> {
        let mut service=lock(&self.service)?;
        if service.is_some() {return Err("已有后台服务所有者，拒绝覆盖活动线程句柄".into());}
        {let mut inner=lock(&self.shared.inner)?;inner.switching=true;inner.capture_running=false;}
        let stop=Arc::new(AtomicBool::new(false));let worker_shared=self.shared.clone();let worker_stop=stop.clone();
        let worker=thread::Builder::new().name("celesvault-archive".into()).spawn(move|| {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||worker_loop(worker_shared.clone(),worker_stop)));
            if result.is_err() && let Ok(mut inner)=lock(&worker_shared.inner) {inner.worker_error=Some("归档后台异常中止；保留意图等待恢复".into());}
        }).map_err(|e| {let error=format!("无法启动归档后台：{e}");self.transition_error(&error);error})?;
        let shared=self.shared.clone();let capture_stop=stop.clone();let (started,ready)=mpsc::channel();
        let capture=thread::Builder::new().name("celesvault-capture".into()).spawn(move|| {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::run(shared.clone(),capture_stop,started)))
                .unwrap_or_else(|_|Err("监测线程异常中止，无法确认 ETW 会话已停止".into()));
            if let Err(error)=&result {shared.capture_error(error.clone());}
            result
        });
        let spawn_error=capture.as_ref().err().map(|error|format!("无法启动监测后台：{error}"));
        *service=Some(Service {stop,capture:capture.ok(),worker:Some(worker),stop_error:None});
        drop(service);
        let result=(|| {
            if let Some(error)=spawn_error {return Err(error);}
            ready.recv_timeout(Duration::from_secs(15)).map_err(|_|lock(&self.shared.inner).ok()
                .and_then(|i|i.capture_error.clone()).unwrap_or_else(||"未能确认 ETW 监测启动".into()))?;
            let mut inner=lock(&self.shared.inner)?;
            if let Some(error)=inner.capture_error.clone().or(inner.worker_error.clone()) {return Err(error);}
            if !inner.capture_running {return Err("ETW 监测启动后已退出".into());}
            inner.switching=false;
            self.shared.wake.notify_all();Ok(())
        })();
        if let Err(error)=result {
            let error=match self.stop_service() {Ok(())=>error,Err(stop)=>format!("{error}；停止替代线程也失败：{stop}")};
            self.transition_error(&error);return Err(error);
        }
        Ok(())
    }
    fn stop_service(&self)->Result<(),String> {self.stop_service_for(Duration::from_secs(15))}
    fn stop_service_for(&self,timeout:Duration)->Result<(),String> {
        let mut owner=lock(&self.service)?;let mut errors=Vec::new();
        if let Some(service)=owner.as_mut() {
            if let Some(error)=&service.stop_error {return Err(error.clone());}
            service.stop.store(true,Ordering::Release);self.shared.wake.notify_all();
            let handles=[service.capture.as_ref().map(|h|h.as_raw_handle()),service.worker.as_ref().map(|h|h.as_raw_handle())];
            for handle in handles.into_iter().flatten() {
                let cancelled=unsafe {windows_sys::Win32::System::IO::CancelSynchronousIo(handle.cast())};
                if cancelled==0 {let code=unsafe {windows_sys::Win32::Foundation::GetLastError()};if code!=windows_sys::Win32::Foundation::ERROR_NOT_FOUND {errors.push(format!("取消后台同步 I/O 失败：Win32={code}"));}}
            }
            let started=Instant::now();
            while service.capture.as_ref().is_some_and(|h|!h.is_finished()) || service.worker.as_ref().is_some_and(|h|!h.is_finished()) {
                if started.elapsed()>=timeout {
                    errors.push("停止后台超时；保留旧线程句柄和停止标记，禁止启动第二组线程，请稍后重启监测".into());
                    let error=errors.join("；");self.shared.capture_error(error.clone());return Err(error);
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        if let Some(service)=owner.as_mut() {
            let mut uncertain=None;
            if let Some(capture)=service.capture.take() {
                match capture.join() {
                    Ok(Ok(()))=>(),Ok(Err(error))=>uncertain=Some(error),
                    Err(_)=>uncertain=Some("监测线程 join 失败，无法确认 ETW 所有权已释放".into()),
                }
            }
            if let Some(worker)=service.worker.take() && worker.join().is_err() {errors.push("归档线程 join 失败".into());}
            if let Some(error)=uncertain {
                service.stop_error=Some(error.clone());errors.push(error);
                let error=errors.join("；");self.shared.capture_error(error.clone());return Err(error);
            }
        }
        *owner=None;
        if errors.is_empty() {Ok(())} else {let error=errors.join("；");self.shared.capture_error(error.clone());Err(error)}
    }
}
#[cfg(test)]
mod policy_tests {
    use super::*;
    use crate::storage::exclusion_tests::{Fixture,setup};
    #[test]
    fn activity_snapshot_keeps_fast_completion_and_orders_the_next_capture() {
        let engine=VaultEngine::new();
        assert!(engine.capture_activity().unwrap().is_none());
        {
            let mut inner=lock(&engine.shared.inner).unwrap();
            inner.record_capture_activity(CaptureActivityKind::Detected);
            let detected=inner.capture_activity.unwrap();
            // 不在接受和完成之间轮询，模拟极快归档；不需要 pending_count 边沿。
            inner.record_capture_activity(CaptureActivityKind::Archived);
            let completed=inner.capture_activity.unwrap();
            assert_eq!(completed.sequence,detected.sequence+1);
            assert!(completed.occurred_at>=detected.occurred_at);
        }
        let completed=engine.capture_activity().unwrap().unwrap();
        assert_eq!(completed.kind,CaptureActivityKind::Archived);
        // 多次读取不刷新时间戳，也不会消费掉其他宿主读取者的信号。
        let reread=engine.capture_activity().unwrap().unwrap();
        assert_eq!(reread.sequence,completed.sequence);
        assert_eq!(reread.occurred_at,completed.occurred_at);
        lock(&engine.shared.inner).unwrap().record_capture_activity(CaptureActivityKind::Detected);
        let next=engine.capture_activity().unwrap().unwrap();
        assert_eq!(next.kind,CaptureActivityKind::Detected);
        assert_eq!(next.sequence,completed.sequence+1);
        assert!(next.occurred_at>=completed.occurred_at);
    }
    fn event(actual:u64,observed:u64)->capture::probe::model::FileEvent {
        capture::probe::model::FileEvent {sequence:1,event_unix_ms:actual,observed_unix_ms:observed,path:"C:\\绝不打开.docx".into(),operation:"open".into(),event_id:64,opcode:64,version:0,header_pid:1,header_tid:1,issuing_tid:None,attributed_pid:None,attribution_source:String::new(),process:None,file_object:None,file_key:None,io_size:None}
    }
    #[test]
    fn rejected_event_never_calls_metadata_opener_and_delayed_etw_does_not_backfill() {
        let mut fixture=Fixture::new();let mut store=fixture.store.take().unwrap();
        let courses=setup(&mut store);let actual=courses[0].start_unix_ms;
        let mut bundle=store.bundle.clone();bundle.capture.excluded_subject_ids=vec!["excluded".into()];
        store.save_config(bundle).unwrap();
        let engine=VaultEngine::new();lock(&engine.shared.inner).unwrap().store=Some(store);
        let settings=lock(&engine.shared.inner).unwrap().store.as_ref().unwrap().settings.clone();
        let blocked=capture::admitted_source(&engine.shared,&settings,&event(actual,actual+1),&AtomicBool::new(false),||->Result<(),String> {panic!("被排除事件不应访问源文件元数据")}).unwrap();
        assert!(blocked.is_none());
        {
            let mut inner=lock(&engine.shared.inner).unwrap();
            let mut bundle=inner.store.as_ref().unwrap().bundle.clone();bundle.capture.excluded_subject_ids.clear();
            inner.store.as_mut().unwrap().recover_and_save_config(bundle).unwrap();
            // 模拟排除解除后的 observed/真实事件双水位。
            inner.capture_resume_after=actual+100;inner.capture_event_after=actual+100;
        }
        let settings=lock(&engine.shared.inner).unwrap().store.as_ref().unwrap().settings.clone();
        for e in [event(actual,actual+200),event(actual+200,actual),event(actual+100,actual+200)] {
            assert!(capture::admitted_source(&engine.shared,&settings,&e,&AtomicBool::new(false),||->Result<(),String> {panic!("旧事件不得回填")}).unwrap().is_none());
        }
        assert_eq!(capture::admitted_source(&engine.shared,&settings,&event(actual+101,actual+101),&AtomicBool::new(false),||Ok(7)).unwrap(),Some(7));
        let inner=lock(&engine.shared.inner).unwrap();
        assert_eq!(inner.store.as_ref().unwrap().connection.query_row("SELECT count(*) FROM day_snapshots",[],|r|r.get::<_,u64>(0)).unwrap(),0);
        drop(inner);drop(engine);
    }
    #[test]
    fn exclusion_commit_waits_for_both_old_threads_and_failure_stays_stopped() {
        let mut fixture=Fixture::new();let mut store=fixture.store.take().unwrap();setup(&mut store);
        let mut bundle=store.bundle.clone();bundle.capture.excluded_subject_ids=vec!["excluded".into()];
        // 人为制造提交失败，避免测试启动真实 ETW；旧策略不得恢复到半应用状态。
        store.connection.execute_batch("DROP TABLE config;").unwrap();
        let engine=VaultEngine::new();{
            let mut inner=lock(&engine.shared.inner).unwrap();inner.store=Some(store);inner.paused=true;
        }
        let stop=Arc::new(AtomicBool::new(false));
        let (release_capture,wait_capture)=mpsc::channel();let (release_worker,wait_worker)=mpsc::channel();
        let capture=thread::spawn(move|| {let _=wait_capture.recv();Ok(())});
        let worker=thread::spawn(move|| {let _=wait_worker.recv();});
        *lock(&engine.service).unwrap()=Some(Service {stop:stop.clone(),capture:Some(capture),worker:Some(worker),stop_error:None});
        let saving=engine.clone();let saved=thread::spawn(move||saving.save_config(bundle));
        let limit=Instant::now();
        while !stop.load(Ordering::Acquire) && limit.elapsed()<Duration::from_secs(3) {thread::yield_now();}
        assert!(stop.load(Ordering::Acquire));
        assert!(lock(&engine.shared.inner).unwrap().store.as_ref().unwrap().settings.excluded_subject_ids.is_empty());
        assert!(!saved.is_finished());release_capture.send(()).unwrap();
        assert!(!saved.is_finished());release_worker.send(()).unwrap();
        assert!(saved.join().unwrap().is_err());
        let inner=lock(&engine.shared.inner).unwrap();
        assert!(inner.paused && inner.switching && !inner.capture_running);
        assert!(inner.worker_error.is_some());
        assert!(inner.store.as_ref().unwrap().settings.excluded_subject_ids.is_empty());
        drop(inner);assert!(lock(&engine.service).unwrap().is_none());drop(engine);
    }
    #[test]
    fn unverified_existing_stage_is_never_deleted_by_retry() {
        use std::io::Write;
        let mut fixture=Fixture::new();let mut store=fixture.store.take().unwrap();let c=setup(&mut store);
        store.accept("C:\\不存在.docx".into(),crate::secure_fs::Identity {volume_serial:1,volume_name:"test".into(),file_id:[1;16],created:1},4,c[0].start_unix_ms).unwrap();
        let task=store.due().unwrap().unwrap();let stage=secure_fs::relative(&store.root,&task.stage_rel).unwrap();
        {let mut file=secure_fs::create_owned(&stage).unwrap();file.write_all(b"keep").unwrap();file.sync_all().unwrap();}
        let engine=VaultEngine::new();lock(&engine.shared.inner).unwrap().store=Some(store);
        let result=archive::execute(&engine.shared,task,&AtomicBool::new(false));
        assert!(result.is_err());assert_eq!(std::fs::read(&stage).unwrap(),b"keep".to_vec());drop(engine);
    }
    #[test]
    fn stop_timeout_keeps_thread_owner_and_pause_and_rejects_second_group() {
        let engine=VaultEngine::new();lock(&engine.shared.inner).unwrap().paused=true;
        let stop=Arc::new(AtomicBool::new(false));let (release,wait)=mpsc::channel();
        let capture=thread::spawn(move|| {let _=wait.recv();Ok(())});
        *lock(&engine.service).unwrap()=Some(Service {stop:stop.clone(),capture:Some(capture),worker:None,stop_error:None});
        assert!(engine.stop_service_for(Duration::ZERO).is_err());
        assert!(stop.load(Ordering::Acquire));assert!(lock(&engine.service).unwrap().as_ref().unwrap().capture.is_some());
        assert!(engine.start_service().is_err());assert!(lock(&engine.shared.inner).unwrap().paused);
        release.send(()).unwrap();engine.stop_service().unwrap();
        assert!(lock(&engine.service).unwrap().is_none());
    }
    #[test]
    fn uncertain_etw_stop_stays_blocked_and_unconfigured_restart_is_honest() {
        let engine=VaultEngine::new();assert!(engine.restart_monitoring().is_err());
        let capture=thread::spawn(||Err("自有 ETW 会话停止失败".into()));
        *lock(&engine.service).unwrap()=Some(Service {stop:Arc::new(AtomicBool::new(false)),capture:Some(capture),worker:None,stop_error:None});
        assert!(engine.stop_service().is_err());assert!(engine.start_service().is_err());
        assert!(lock(&engine.service).unwrap().as_ref().unwrap().stop_error.is_some());
    }
}

impl Drop for VaultEngine {fn drop(&mut self) {self.shutdown();}}
fn worker_loop(shared:Arc<Shared>,stop:Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let task=(|| {let mut inner=lock(&shared.inner)?;if inner.switching {return Ok(None);}inner.store.as_mut().ok_or("数据目录尚未配置")?.due()})();
        match task {
            Ok(Some(task))=>{
                match archive::execute(&shared,task.clone(),&stop) {
                    Ok(())=>{
                        if let Ok(mut inner)=lock(&shared.inner) {inner.record_capture_activity(CaptureActivityKind::Archived);}
                    },
                    Err(error)=>{
                        // 停止期间不把取消错误转成失败/重试；join 后统一恢复 copying 意图。
                        let failed=(|| {
                            let inner=lock(&shared.inner)?;
                            if stop.load(Ordering::Acquire) || inner.switching || inner.shutdown {return Ok(());}
                            inner.store.as_ref().ok_or("数据目录尚未配置")?.fail(&task,&error.message,error.terminal)
                        })();
                        if let Err(error)=failed {if let Ok(mut inner)=lock(&shared.inner) {inner.worker_error=Some(bounded(error));}}
                    },
                }
            }
            Ok(None)=>{let guard=shared.wait.lock().unwrap_or_else(|p|p.into_inner());let _wait=shared.wake.wait_timeout(guard,Duration::from_millis(250));}
            Err(error)=>{if let Ok(mut inner)=lock(&shared.inner) {inner.worker_error=Some(bounded(error));}break;}
        }
    }
}
