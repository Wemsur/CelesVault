use crate::api::*;
use std::{path::PathBuf,sync::Arc};

pub struct ArchiveLease {pub path:PathBuf}
impl ArchiveLease {pub fn display_path(&self)->Result<PathBuf,String> {Err(unavailable())}}
pub struct VaultEngine;
impl VaultEngine {
    pub fn new()->Arc<Self> {Arc::new(Self)}
    pub fn status(&self)->Result<EngineStatus,String> {
        Ok(EngineStatus {data_root:None,monitor_state:"error".into(),monitor_detail:"真实监测/归档仅支持 Windows，未回退到浏览器模拟".into(),pending_count:0,failed_count:0,has_errors:true,health:vec!["当前平台不支持 Windows ETW 与卷/文件身份校验".into()]})
    }
    pub fn cleanup_and_restart_monitoring(&self,_cleanup:impl FnOnce()->Result<u32,String>)->Result<EngineStatus,String>{Err(unavailable())}
    pub fn restart_monitoring(&self)->Result<EngineStatus,String> {Err(unavailable())}
    pub fn configure_root(&self,_path:String)->Result<EngineStatus,String> {Err(unavailable())}
    pub fn config(&self)->Result<ConfigBundle,String> {Err(unavailable())}
    pub fn save_config(&self,_bundle:ConfigBundle)->Result<ConfigBundle,String> {Err(unavailable())}
    pub fn import_config(&self,_text:String)->Result<ConfigBundle,String> {Err(unavailable())}
    pub fn export_config(&self)->Result<String,String> {Err(unavailable())}
    pub fn import_timetable(&self,_text:String,_format:String)->Result<Timetable,String> {Err(unavailable())}
    pub fn export_timetable(&self,_format:String)->Result<String,String> {Err(unavailable())}
    pub fn courses(&self,_date:String)->Result<Vec<CourseView>,String> {Err(unavailable())}
    pub fn change_course(&self,_course_id:String,_subject_id:String)->Result<(),String> {Err(unavailable())}
    pub fn list_today(&self)->Result<Vec<ArchiveItem>,String> {Err(unavailable())}
    pub fn library(&self,_query:LibraryQuery)->Result<LibraryPage,String> {Err(unavailable())}
    pub fn detail(&self,_id:String)->Result<ArchiveDetail,String> {Err(unavailable())}
    pub fn edit_archive(&self,_id:String,_name:String,_course_id:Option<String>)->Result<(),String> {Err(unavailable())}
    pub fn move_usage(&self,_id:String,_from_course_id:String,_to_course_id:String)->Result<(),String> {Err(unavailable())}
    pub fn favorite_save(&self,_folder:FixedFolder)->Result<(),String> {Err(unavailable())}
    pub fn favorite_remove(&self,_id:String)->Result<(),String> {Err(unavailable())}
    pub fn trash(&self,_id:String)->Result<(),String> {Err(unavailable())}
    pub fn trash_list(&self)->Result<Vec<ArchiveDetail>,String> {Err(unavailable())}
    pub fn restore(&self,_id:String)->Result<(),String> {Err(unavailable())}
    pub fn purge(&self,_id:String)->Result<(),String> {Err(unavailable())}
    pub fn busy_volumes(&self)->Result<Vec<String>,String> {Err(unavailable())}
    pub fn thumbnail(&self,_id:String)->Result<Option<String>,String> {Err(unavailable())}
    pub fn pause(&self,_paused:bool)->Result<EngineStatus,String> {Err(unavailable())}
    pub fn dismiss_failed(&self,_task_id:String)->Result<(),String> {Err(unavailable())}
    pub fn retry(&self,_task_id:String)->Result<(),String> {Err(unavailable())}
    pub fn archived_path(&self,_id:String)->Result<PathBuf,String> {Err(unavailable())}
    pub fn lease_archive(&self,_id:String)->Result<ArchiveLease,String> {Err(unavailable())}
    pub fn get_capture_settings(&self)->Result<CaptureSettings,String> {Err(unavailable())}
    pub fn set_capture_settings(&self,_settings:CaptureSettings)->Result<(),String> {Err(unavailable())}
    pub fn source_defaults(&self)->Result<Vec<SourceRule>,String> {Err(unavailable())}
    pub fn shutdown(&self) {}
}
fn unavailable()->String {"此平台不支持真实 Windows 监测/归档能力；未模拟成功".into()}
