use crate::{api::{CaptureSettings, SourceRule}, engine::{lock, Shared}, secure_fs, storage};
use std::{path::{Path,PathBuf}, sync::{Arc,Mutex,RwLock,atomic::{AtomicBool,Ordering},mpsc}, thread, time::{Duration,Instant}};
use probe::{etw::{Collector,TraceSession}, filter::{self,Scope}, model::ProcessInfo, windows};
pub mod probe;

fn forbidden_process(name: &str) -> bool {
    ["system","system.exe","explorer.exe","dllhost.exe","prevhost.exe","svchost.exe","searchindexer.exe"]
        .contains(&name.to_ascii_lowercase().as_str())
}
pub fn validate_settings(mut settings: CaptureSettings) -> Result<CaptureSettings,String> {
    if !["strict","loose"].contains(&settings.mode.as_str()) { return Err("监测模式只能是 strict 或 loose".into()); }
    for count in [settings.process_names.len(),settings.process_paths.len(),settings.wps_install_paths.len(),settings.sources.len(),settings.extensions.len(),settings.excluded_paths.len()] {
        if count > 128 { return Err("每类监测规则最多 128 项".into()); }
    }
    for name in &mut settings.process_names {
        *name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.contains(['\\','/',':']) || !name.ends_with(".exe") || forbidden_process(name) {
            return Err("主程序规则必须是明确的应用 exe 名；不能使用 Explorer/dllhost/System 等通用来源".into());
        }
    }
    for ext in &mut settings.extensions {
        *ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        if ext.is_empty() || ext.len()>16 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("扩展名必须是 1–16 个英文或数字".into());
        }
    }
    for path in settings.process_paths.iter().chain(settings.wps_install_paths.iter()).chain(settings.excluded_paths.iter()).chain(settings.sources.iter().map(|s| &s.path)) {
        secure_fs::lexical_path(path)?;
    }
    for path in &settings.process_paths {
        let name = path.rsplit(['\\','/']).next().unwrap_or("");
        if forbidden_process(name) || !name.to_ascii_lowercase().ends_with(".exe") { return Err("程序路径不能指向通用系统进程".into()); }
    }
    for path in &settings.wps_install_paths {
        if filter::normalize(path).len() <= 3 { return Err("WPS 安装目录不能是整个盘根".into()); }
    }
    settings.process_names.sort(); settings.process_names.dedup();
    settings.extensions.sort(); settings.extensions.dedup();
    Ok(settings)
}

pub fn source_defaults() -> Vec<SourceRule> {
    let mut roots = windows::default_roots();
    for drive in windows::drives() {
        if drive.selected_automatically || drive.root.eq_ignore_ascii_case("D:\\") { roots.push(drive.root); }
    }
    roots.into_iter().map(|path| SourceRule {path,recursive:true}).collect()
}

fn scope(settings: &CaptureSettings, root: &Path, drives: &[probe::model::DriveInfo]) -> Scope {
    let mut roots: Vec<String> = settings.sources.iter().map(|s| s.path.clone()).collect();
    if settings.include_default_sources {
        roots.extend(windows::default_roots());
        roots.extend(drives.iter().filter(|d| d.root.eq_ignore_ascii_case("D:\\")).map(|d|d.root.clone()));
    }
    if settings.include_usb { roots.extend(drives.iter().filter(|d|d.selected_automatically).map(|d|d.root.clone())); }
    let mut excluded = settings.excluded_paths.clone();
    excluded.push(root.to_string_lossy().into_owned());
    for name in ["TEMP","TMP"] { if let Ok(temp)=std::env::var(name) { excluded.push(temp); } }
    if let Ok(windir)=std::env::var("WINDIR") { excluded.push(format!("{windir}\\Temp")); }
    let mut scope = Scope::new(roots,excluded,&settings.extensions.join(","));
    windows::update_scope(&mut scope,drives,false);
    scope
}
fn source_allowed(path: &str, settings: &CaptureSettings, active: &Scope) -> bool {
    if !active.matches(path) || temporary(path) { return false; }
    let explicit = settings.sources.iter().any(|s| filter::under(path,&s.path) &&
        (s.recursive || Path::new(path).parent().is_some_and(|p| filter::normalize(&p.to_string_lossy()).eq_ignore_ascii_case(&filter::normalize(&s.path)))));
    let implicit = active.roots.iter().filter(|r| !settings.sources.iter().any(|s| filter::normalize(r).eq_ignore_ascii_case(&filter::normalize(&s.path))))
        .any(|r|filter::under(path,r));
    explicit || implicit
}
pub fn temporary(path: &str) -> bool {
    let name = path.rsplit(['\\','/']).next().unwrap_or(path).to_ascii_lowercase();
    name.starts_with("~$") || name.starts_with(".~") || name.starts_with('~')
        || name.ends_with(".tmp") || name.ends_with(".temp") || name.ends_with(".part")
        || path.split(['\\','/']).any(|p| ["temp","tmp"].contains(&p.to_ascii_lowercase().as_str()))
}
fn wps_root(path: &str) -> Option<String> {
    let path = PathBuf::from(path);
    // 从实际 WPS 主程序路径识别安装树，不猜盘根/任意父目录。
    path.ancestors().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("WPS Office")))
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| path.ancestors().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("office6")))
            .map(|p|p.to_string_lossy().into_owned()))
}
fn process_allowed(process: &ProcessInfo, settings: &CaptureSettings, installs: &[String]) -> bool {
    let Some(path) = process.executable_path.as_deref() else { return false; };
    let name = path.rsplit(['\\','/']).next().unwrap_or("").to_ascii_lowercase();
    if forbidden_process(&name) { return false; }
    let main = settings.process_names.iter().any(|p|p.eq_ignore_ascii_case(&name))
        || settings.process_paths.iter().any(|p|filter::normalize(p).eq_ignore_ascii_case(&filter::normalize(path)));
    main || (settings.mode=="loose" && installs.iter().any(|root|filter::under(path,root)))
}
fn install_paths(settings: &CaptureSettings) -> Vec<String> {
    let mut paths = settings.wps_install_paths.clone();
    let (processes,_) = windows::snapshot();
    for pid in processes.keys() {
        if let Some(p)=windows::process_info(*pid) {
            if process_allowed(&p,&CaptureSettings {mode:"strict".into(),..settings.clone()},&[])
                && let Some(root)=p.executable_path.as_deref().and_then(wps_root)
                && !paths.iter().any(|p|filter::normalize(p).eq_ignore_ascii_case(&filter::normalize(&root))) {
                    paths.push(root);
            }
        }
        if paths.len()>=128 { break; }
    }
    paths
}

/// 原始事件队列严格有界；ETW 回调只解析/关联/try_send，不做摘要、复制或 DB。
pub fn run(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let initial=(|| {
        let inner=lock(&shared.inner)?;
        let store=inner.store.as_ref().ok_or("尚未配置数据目录")?;
        store.validate()?;
        Ok::<_,String>((store.settings.clone(),store.root.clone(),store.display_root()?))
    })();
    let (settings,root,mut display_root)=match initial {
        Ok(value)=>value,Err(error)=>{shared.capture_error(error);return;}
    };
    windows::STOP_REQUESTED.store(false,Ordering::Release);
    let mut settings=settings;
    let mut installs=install_paths(&settings);
    let active=Arc::new(RwLock::new(scope(&settings,&display_root,&windows::drives())));
    let (tx,rx)=mpsc::sync_channel(512);
    let collector=Arc::new(Mutex::new(Collector::new(active.clone(),tx)));
    let mut trace=match TraceSession::start(collector.clone()) {
        Ok(trace)=>trace,
        Err(error)=>{shared.capture_error(error);return;}
    };
    shared.capture_started();
    let mut refreshed=Instant::now();
    let mut unknown=0u64;
    let mut rejected_identity=0u64;
    while !stop.load(Ordering::Acquire) {
        if windows::STOP_REQUESTED.load(Ordering::Acquire) || trace.consumer_finished() {
            shared.capture_error("ETW 消费线程已中止；未模拟监测成功".into());break;
        }
        if refreshed.elapsed()>=Duration::from_secs(2) {
            let refreshed_root=(|| {
                let inner=lock(&shared.inner)?;let store=inner.store.as_ref().ok_or("数据目录未配置")?;
                store.validate()?;Ok::<_,String>((store.settings.clone(),store.display_root()?))
            })();
            match refreshed_root {Ok((current,path))=>{settings=current;display_root=path;},Err(error)=>{shared.capture_error(error);break;}}
            let drives=windows::drives();
            if let Ok(mut s)=active.write() { *s=scope(&settings,&display_root,&drives); }
            installs=install_paths(&settings);
            let counters=collector.lock().unwrap_or_else(|p|p.into_inner()).counters.clone();
            let health=trace.health();
            if health.query_code != 0 {shared.capture_error(format!("ETW 会话健康查询失败：Win32={}",health.query_code));break;}
            shared.capture_health(counters,health,unknown,rejected_identity);
            refreshed=Instant::now();
        }
        let event=match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(e)=>e,Err(mpsc::RecvTimeoutError::Timeout)=>continue,Err(_)=>break,
        };
        if ![64,67].contains(&event.opcode) { continue; }
        if lock(&shared.inner).map(|i|i.paused || i.switching || event.observed_unix_ms<=i.capture_resume_after).unwrap_or(true) { continue; }
        let Some(process)=&event.process else {unknown=unknown.saturating_add(1);continue;};
        if process.executable_path.is_none() {unknown=unknown.saturating_add(1);continue;}
        if !process_allowed(process,&settings,&installs) {continue;}
        if !active.read().map(|s|source_allowed(&event.path,&settings,&s)).unwrap_or(false) {continue;}
        let data_valid=(|| {lock(&shared.inner)?.store.as_ref().ok_or("数据目录未配置")?.validate()})();
        if let Err(error)=data_valid {shared.capture_error(error);break;}
        let opened=match secure_fs::local_path(&event.path).and_then(|p|secure_fs::Opened::source(&p,true)) {
            Ok(file)=>file,Err(error)=>{rejected_identity=rejected_identity.saturating_add(1);shared.note_capture_error(error);continue;}
        };
        let candidate_guid=match secure_fs::guid_path(&opened.file) {
            Ok(path)=>path,Err(error)=>{rejected_identity=rejected_identity.saturating_add(1);shared.note_capture_error(error);continue;}
        };
        if filter::under(&candidate_guid.to_string_lossy(),&root.to_string_lossy()) {continue;}
        let snapshot=match secure_fs::snapshot(&opened.file) {
            Ok(info)=>info,Err(error)=>{rejected_identity=rejected_identity.saturating_add(1);shared.note_capture_error(error);continue;}
        };
        drop(opened);
        // ETW 原始有效时间决定归属，不用 observed/copy 完成时刻；失效时间不猜。
        if event.event_unix_ms==0 || storage::event_date(event.event_unix_ms).is_err() {
            rejected_identity=rejected_identity.saturating_add(1);continue;
        }
        let result=(|| {
            let mut inner=lock(&shared.inner)?;
            if inner.paused || inner.switching || event.observed_unix_ms<=inner.capture_resume_after {return Ok(false);}
            inner.store.as_mut().ok_or("尚未配置数据目录")?.accept(event.path,snapshot.identity,snapshot.size,event.event_unix_ms)
        })();
        match result {Ok(true)=>shared.wake.notify_all(),Ok(false)=>(),Err(error)=>shared.note_capture_error(error)}
    }
    let health=trace.stop(); // 只停止独立 GUID_NULL 会话，并回收 consumer 所有权。
    let counters=collector.lock().unwrap_or_else(|p|p.into_inner()).counters.clone();
    shared.capture_health(counters,health,unknown,rejected_identity);
}
