use crate::{ArchiveItem,CaptureSettings,EngineStatus,SourceRule};
use std::{path::PathBuf,sync::Arc};

pub struct ArchiveLease {pub path:PathBuf}
pub struct VaultEngine;
impl VaultEngine {
    pub fn new()->Arc<Self> {Arc::new(Self)}
    pub fn status(&self)->Result<EngineStatus,String> {
        Ok(EngineStatus {data_root:None,monitor_state:"error".into(),monitor_detail:"真实监测/归档仅支持 Windows，未回退到浏览器模拟".into(),pending_count:0,failed_count:0,has_errors:true,health:vec!["当前平台不支持 Windows ETW 与卷/文件身份校验".into()]})
    }
    pub fn configure_root(&self,_path:String)->Result<EngineStatus,String> {Err(unavailable())}
    pub fn list_today(&self)->Result<Vec<ArchiveItem>,String> {Err(unavailable())}
    pub fn pause(&self,_paused:bool)->Result<EngineStatus,String> {Err(unavailable())}
    pub fn retry(&self,_task_id:String)->Result<(),String> {Err(unavailable())}
    pub fn archived_path(&self,_id:String)->Result<PathBuf,String> {Err(unavailable())}
    pub fn lease_archive(&self,_id:String)->Result<ArchiveLease,String> {Err(unavailable())}
    pub fn get_capture_settings(&self)->Result<CaptureSettings,String> {Err(unavailable())}
    pub fn set_capture_settings(&self,_settings:CaptureSettings)->Result<(),String> {Err(unavailable())}
    pub fn source_defaults(&self)->Result<Vec<SourceRule>,String> {Err(unavailable())}
    pub fn shutdown(&self) {}
}
fn unavailable()->String {"此平台不支持真实 Windows 监测/归档能力".into()}
