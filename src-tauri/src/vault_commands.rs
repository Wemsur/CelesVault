use std::{collections::HashSet, sync::Arc};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use vault_core::{AppSettings, ArchiveDetail, ArchiveItem, ArchiveLease, CaptureSettings, ConfigBundle, CourseView, EngineStatus, FixedFolder, LibraryPage, LibraryQuery, Timetable, VaultEngine};
use crate::{services::Services, fs_jobs::{ConflictResolution, CopyJob, CopyPlan, CopyRequest, DirectoryView, DriveInfo, ExportSource, FileRecovery}};

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
	tauri::async_runtime::spawn_blocking(work).await.map_err(|error| format!("后台操作未完成：{error}"))?
}
fn sources(engine: &VaultEngine, ids: Vec<String>) -> Result<(Vec<ExportSource>, Vec<ArchiveLease>), String> {
	if ids.is_empty() || ids.len() > 1000 { return Err("请选择1–1000份已归档资料".into()); }
	let mut seen = HashSet::new(); let mut result = Vec::new(); let mut leases = Vec::new();
	for id in ids {
		if !seen.insert(id.clone()) { continue; }
		let detail = engine.detail(id.clone())?;
		if detail.deleted || detail.item.state != "available" { return Err("只能导出已成功保存且不在回收区的资料".into()); }
		let lease = engine.lease_archive(id.clone())?;
		result.push(ExportSource { id, display_name: detail.item.display_name, path: lease.path.clone() }); leases.push(lease);
	}
	Ok((result, leases))
}
fn require_live(services: &Services) -> Result<(), String> {
	if services.stopping.load(std::sync::atomic::Ordering::Acquire) { Err("应用正在退出，不接受新的操作".into()) } else { Ok(()) }
}
fn favorite_device(drive: &DriveInfo) -> Option<String> {
	if drive.identity_reliable { if let Some(id) = &drive.device_id { return Some(id.clone()); } }
	if !drive.is_usb { Some(format!("local-volume:{}", drive.volume_id)) } else { None }
}
fn validate_favorite(folder: &FixedFolder, drives: &[DriveInfo]) -> Result<(), String> {
	if !drives.iter().any(|drive| drive.volume_id.eq_ignore_ascii_case(&folder.volume_id) && favorite_device(drive).as_deref() == Some(folder.device_id.as_str())) {
		return Err("不能确认固定夹对应的设备和分区；请插入原设备或主动重新定位".into());
	}
	Ok(())
}

#[tauri::command]
pub async fn vault_status(services: State<'_, Services>) -> Result<EngineStatus, String> {
	let engine = services.engine.clone(); let mut status = blocking(move || engine.status()).await?;
	if let Some(error) = services.startup_error.lock().map_err(|_| "状态锁不可用")?.clone() { status.health.push(error); status.has_errors = true; }
	if let Some(error) = services.files.watch_error()? { status.health.push(format!("目录监听：{error}")); status.has_errors = true; }
	Ok(status)
}
#[tauri::command]
pub async fn choose_data_root(app: tauri::AppHandle) -> Result<Option<EngineStatus>, String> {
	blocking(move || {
		let executable = std::env::current_exe().map_err(|error| error.to_string())?;
		let default = executable.parent().ok_or("无法读取程序目录")?.join("data");
		let selected = app.dialog().file().set_title("选择不受还原影响的持久目录（默认程序同级 data）").set_directory(&default).blocking_pick_folder();
		let Some(selected) = selected else { return Ok(None); };
		let path = selected.into_path().map_err(|error| error.to_string())?.to_string_lossy().into_owned();
		let services = app.state::<Services>(); let _mutation = services.mutation.lock().map_err(|_| "目录切换锁不可用")?; require_live(&services)?;
		if services.files.jobs()?.iter().any(|job| job.state == "running") { return Err("复制任务仍在运行，不能更换持久目录".into()); }
		services.files.unwatch_directory()?;
		let status = services.engine.configure_root(path)?;
		if let Some(root) = &status.data_root { crate::locator::save(root).map_err(|error| format!("资料库已初始化，但目录定位未保存，下次需重新选择：{error}"))?; }
		*services.startup_error.lock().map_err(|_| "状态锁不可用")? = None;
		let _ = app.emit("vault:config-changed", ()); Ok(Some(status))
	}).await
}
#[tauri::command]
pub async fn vault_today(services: State<'_, Services>) -> Result<Vec<ArchiveItem>, String> { let engine=services.engine.clone(); blocking(move || engine.list_today()).await }
#[tauri::command]
pub async fn vault_pause(services: State<'_, Services>, paused: bool) -> Result<EngineStatus, String> { let engine=services.engine.clone(); blocking(move || engine.pause(paused)).await }
#[tauri::command]
pub async fn restart_monitoring(app: tauri::AppHandle) -> Result<EngineStatus, String> {
	blocking(move || {
		let services = app.state::<Services>();
		let _mutation = services.mutation.lock().map_err(|_| "监测操作锁不可用")?;
		require_live(&services)?;
		let status = services.engine.restart_monitoring()?;
		*services.startup_error.lock().map_err(|_| "状态锁不可用")? = None;
		Ok(status)
	}).await
}
#[tauri::command]
pub async fn vault_retry(services: State<'_, Services>, task_id: String) -> Result<(), String> { let engine=services.engine.clone(); blocking(move || engine.retry(task_id)).await }
#[tauri::command]
pub async fn capture_settings(services: State<'_, Services>) -> Result<CaptureSettings, String> { let engine=services.engine.clone(); blocking(move || engine.get_capture_settings()).await }
#[tauri::command]
pub async fn set_capture_settings(app: tauri::AppHandle, settings: CaptureSettings) -> Result<(), String> {
	blocking(move || { let services=app.state::<Services>(); let _mutation=services.mutation.lock().map_err(|_|"设置锁不可用")?; require_live(&services)?; services.engine.set_capture_settings(settings)?; let _=app.emit("vault:config-changed",()); Ok(()) }).await
}
#[tauri::command]
pub async fn vault_config(services: State<'_, Services>) -> Result<ConfigBundle, String> { let engine=services.engine.clone(); blocking(move || { if engine.status()?.data_root.is_none() {Ok(ConfigBundle::default())} else {engine.config()} }).await }
#[tauri::command]
pub async fn save_vault_config(app: tauri::AppHandle, config: ConfigBundle) -> Result<ConfigBundle, String> {
	blocking(move || {
		let services=app.state::<Services>(); let _mutation=services.mutation.lock().map_err(|_|"设置锁不可用")?; require_live(&services)?;
		let previous=services.engine.config()?; let drives=services.files.list_drives()?;
		for folder in &config.favorites { if !previous.favorites.iter().any(|old|old.id==folder.id && old.device_id==folder.device_id && old.volume_id==folder.volume_id && old.relative_path==folder.relative_path) {validate_favorite(folder,&drives)?;} }
		let saved=services.engine.save_config(config)?; let _=app.emit("vault:config-changed",()); Ok(saved)
	}).await
}
#[tauri::command]
pub async fn save_app_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<ConfigBundle, String> {
	blocking(move || {let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"界面状态锁不可用")?;require_live(&services)?;let mut current=services.engine.config()?;current.app=settings;let saved=services.engine.save_config(current)?;let _=app.emit("vault:config-changed",());Ok(saved)}).await
}
#[tauri::command]
pub async fn vault_courses(services: State<'_, Services>, date: String) -> Result<Vec<CourseView>, String> {let engine=services.engine.clone();blocking(move || {if engine.status()?.data_root.is_none(){Ok(Vec::new())}else{engine.courses(date)}}).await}
#[tauri::command]
pub async fn change_course(services: State<'_, Services>, course_id: String, subject_id: String) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.change_course(course_id,subject_id)).await}
#[tauri::command]
pub async fn library_query(services: State<'_, Services>, query: LibraryQuery) -> Result<LibraryPage, String> {let engine=services.engine.clone();blocking(move || {if engine.status()?.data_root.is_none(){Ok(LibraryPage{rows:Vec::new(),total:0})}else{engine.library(query)}}).await}
#[tauri::command]
pub async fn archive_detail(services: State<'_, Services>, id: String) -> Result<ArchiveDetail, String> {let engine=services.engine.clone();blocking(move ||engine.detail(id)).await}
#[tauri::command]
pub async fn edit_archive(services: State<'_, Services>, id: String, name: String, course_id: Option<String>) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.edit_archive(id,name,course_id)).await}
#[tauri::command]
pub async fn move_usage(services: State<'_, Services>, id: String, from_course_id: String, to_course_id: String) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.move_usage(id,from_course_id,to_course_id)).await}
#[tauri::command]
pub async fn save_favorite(app: tauri::AppHandle, folder: FixedFolder) -> Result<(), String> {
	blocking(move || {let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"固定夹锁不可用")?;require_live(&services)?;let previous=services.engine.config()?;if !previous.favorites.iter().any(|old|old.id==folder.id && old.device_id==folder.device_id && old.volume_id==folder.volume_id && old.relative_path==folder.relative_path){validate_favorite(&folder,&services.files.list_drives()?)?;}services.engine.favorite_save(folder)?;let _=app.emit("vault:config-changed",());Ok(())}).await
}
#[tauri::command]
pub async fn remove_favorite(app: tauri::AppHandle, id: String) -> Result<(), String> {blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"固定夹锁不可用")?;require_live(&services)?;services.engine.favorite_remove(id)?;let _=app.emit("vault:config-changed",());Ok(())}).await}
#[tauri::command]
pub async fn trash_archive(services: State<'_, Services>, id: String) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.trash(id)).await}
#[tauri::command]
pub async fn trash_list(services: State<'_, Services>) -> Result<Vec<ArchiveDetail>, String> {let engine=services.engine.clone();blocking(move || {if engine.status()?.data_root.is_none(){Ok(Vec::new())}else{engine.trash_list()}}).await}
#[tauri::command]
pub async fn restore_archive(services: State<'_, Services>, id: String) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.restore(id)).await}
#[tauri::command]
pub async fn purge_archive(services: State<'_, Services>, id: String) -> Result<(), String> {let engine=services.engine.clone();blocking(move ||engine.purge(id)).await}
#[tauri::command]
pub async fn archive_thumbnail(services: State<'_, Services>, id: String) -> Result<Option<String>, String> {let engine=services.engine.clone();blocking(move ||engine.thumbnail(id)).await}
#[tauri::command]
pub async fn import_timetable(services: State<'_, Services>, text: String, format: String) -> Result<Timetable, String> {let engine=services.engine.clone();blocking(move ||engine.import_timetable(text,format)).await}
#[tauri::command]
pub async fn import_config(services: State<'_, Services>, text: String) -> Result<ConfigBundle, String> {let engine=services.engine.clone();blocking(move ||engine.import_config(text)).await}
#[tauri::command]
pub async fn export_timetable(services: State<'_, Services>, format: String) -> Result<String, String> {let engine=services.engine.clone();blocking(move ||engine.export_timetable(format)).await}
#[tauri::command]
pub async fn export_config(services: State<'_, Services>) -> Result<String, String> {let engine=services.engine.clone();blocking(move ||engine.export_config()).await}
#[tauri::command]
pub async fn open_archive(app: tauri::AppHandle, services: State<'_, Services>, id: String) -> Result<(), String> {
	let engine=services.engine.clone();blocking(move ||{let lease=engine.lease_archive(id)?;let path=lease.display_path()?;app.opener().open_path(path.to_string_lossy().into_owned(),None::<&str>).map_err(|error|error.to_string())}).await
}
#[tauri::command]
pub async fn list_drives(services: State<'_, Services>) -> Result<Vec<DriveInfo>, String> {let files=services.files.clone();blocking(move ||files.list_drives()).await}
#[tauri::command]
pub async fn browse_directory(services: State<'_, Services>, path: String, show_hidden: Option<bool>) -> Result<DirectoryView, String> {let files=services.files.clone();let engine=services.engine.clone();blocking(move ||files.browse_options(path,engine.status()?.data_root,show_hidden.unwrap_or(false))).await}
#[derive(serde::Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct ExportRequest {ids:Vec<String>,target_path:String,target_identity:String,conflict:String}
#[tauri::command]
pub async fn export_archives(app: tauri::AppHandle, request: ExportRequest) -> Result<CopyJob, String> {
	blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"导出锁不可用")?;require_live(&services)?;let (sources,leases)=sources(&services.engine,request.ids)?;let job=services.files.start_copy(sources,CopyRequest{target_path:request.target_path,target_identity:request.target_identity,conflict:request.conflict},services.engine.status()?.data_root)?;drop(leases);Ok(job)}).await
}
#[tauri::command]
pub async fn preview_copy(app: tauri::AppHandle, request: ExportRequest) -> Result<CopyPlan, String> {
	blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"复制预览锁不可用")?;require_live(&services)?;let(sources,leases)=sources(&services.engine,request.ids)?;let plan=services.files.preview_copy(sources,CopyRequest{target_path:request.target_path,target_identity:request.target_identity,conflict:request.conflict},services.engine.status()?.data_root)?;drop(leases);Ok(plan)}).await
}
#[tauri::command]
pub async fn start_copy_plan(app: tauri::AppHandle, plan_id: String, resolutions: Vec<ConflictResolution>) -> Result<CopyJob, String> {
	blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"复制批准锁不可用")?;require_live(&services)?;let ids=services.files.plan_ids(plan_id.clone())?;let(sources,leases)=sources(&services.engine,ids)?;let job=services.files.start_plan(sources,plan_id,resolutions,services.engine.status()?.data_root)?;drop(leases);Ok(job)}).await
}
#[tauri::command]
pub fn copy_jobs(services: State<'_, Services>) -> Result<Vec<CopyJob>, String> {services.files.jobs()}
#[tauri::command]
pub fn cancel_copy(services: State<'_, Services>, job_id: String) -> Result<(), String> {services.files.cancel(job_id)}
#[tauri::command]
pub async fn create_folder(app: tauri::AppHandle, path: String, identity: String, name: String) -> Result<(), String> {blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"文件操作锁不可用")?;require_live(&services)?;services.files.create_folder(path,identity,name,services.engine.status()?.data_root)}).await}
#[tauri::command]
pub async fn rename_entry(app: tauri::AppHandle, path: String, identity: String, name: String) -> Result<(), String> {blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"文件操作锁不可用")?;require_live(&services)?;services.files.rename_entry(path,identity,name,services.engine.status()?.data_root)}).await}
#[tauri::command]
pub async fn recycle_entry(app: tauri::AppHandle, path: String, identity: String) -> Result<(), String> {blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"文件操作锁不可用")?;require_live(&services)?;services.files.recycle_entry(path,identity,services.engine.status()?.data_root)}).await}
#[tauri::command]
pub async fn open_browser_entry(app: tauri::AppHandle, services: State<'_, Services>, path: String, identity: String) -> Result<(), String> {let files=services.files.clone();let engine=services.engine.clone();blocking(move ||{let lease=files.lease_browser_entry(path,identity,engine.status()?.data_root)?;app.opener().open_path(lease.path.to_string_lossy().into_owned(),None::<&str>).map_err(|error|error.to_string())}).await}
#[tauri::command]
pub async fn reveal_browser_entry(app: tauri::AppHandle, services: State<'_, Services>, path: String, identity: String) -> Result<(), String> {let files=services.files.clone();let engine=services.engine.clone();blocking(move ||{let lease=files.lease_browser_entry(path,identity,engine.status()?.data_root)?;app.opener().reveal_item_in_dir(&lease.path).map_err(|error|error.to_string())}).await}
#[tauri::command]
pub async fn watch_directory(app: tauri::AppHandle, path: String, identity: String) -> Result<(), String> {
	blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"监听切换锁不可用")?;require_live(&services)?;let handle=app.clone();let callback=Arc::new(move ||{let _=handle.emit("vault:directory-changed",());});services.files.watch_directory(path,identity,services.engine.status()?.data_root,callback)}).await
}
#[tauri::command]
pub async fn unwatch_directory(services: State<'_, Services>) -> Result<(), String> {let files=services.files.clone();blocking(move ||files.unwatch_directory()).await}
#[tauri::command]
pub async fn file_recoveries(services: State<'_, Services>, path: String, identity: String) -> Result<Vec<FileRecovery>, String> {let files=services.files.clone();let engine=services.engine.clone();blocking(move ||files.file_recoveries(path,identity,engine.status()?.data_root)).await}
#[tauri::command]
pub async fn restore_file_recovery(app: tauri::AppHandle, path: String, identity: String, recovery_id: String) -> Result<(), String> {blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"恢复操作锁不可用")?;require_live(&services)?;services.files.restore_file_recovery(path,identity,recovery_id,services.engine.status()?.data_root)}).await}
#[tauri::command]
pub async fn eject_device(app: tauri::AppHandle, device_id: String) -> Result<(), String> {
	blocking(move ||{let services=app.state::<Services>();let _mutation=services.mutation.lock().map_err(|_|"设备弹出锁不可用")?;require_live(&services)?;services.files.unwatch_directory()?;let mut blocked=services.engine.busy_volumes()?;blocked.extend(services.files.busy_volumes()?);crate::devices::eject(device_id,blocked)}).await
}
fn next_text_export_name(path: &std::path::Path, first: u32) -> String {
	let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("CelesVault-导出");
	let extension = path.extension().and_then(|value| value.to_str())
		.map(|value| format!(".{value}")).unwrap_or_default();
	// 这里只生成对话框建议；真正写入仍使用 create_new，避免检查后名称被占用。
	for number in first..first + 1000 {
		let name = format!("{stem} ({number}){extension}");
		if matches!(path.with_file_name(&name).try_exists(), Ok(false)) { return name; }
	}
	format!("{stem} ({}){extension}", first + 1000)
}

#[tauri::command]
pub async fn save_text_export(app: tauri::AppHandle, text: String, suggested_name: String) -> Result<bool, String> {
	blocking(move ||{
		use std::io::Write;
		if text.len()>2*1024*1024 {return Err("导出文本超过2MiB限制".into());}
		if suggested_name.contains(['\\','/']) || suggested_name.len()>200 {return Err("导出文件名非法".into());}
		let mut next_name = suggested_name;
		let mut next_directory = None;
		for attempt in 0..32 {
			let mut dialog = app.dialog().file()
				.set_title("另存配置、课表或诊断（不会覆盖已有文件）")
				.set_file_name(&next_name);
			if let Some(parent) = &next_directory { dialog = dialog.set_directory(parent); }
			let Some(selected) = dialog.blocking_save_file() else { return Ok(false); };
			let path = selected.into_path().map_err(|error| error.to_string())?;
			match std::fs::OpenOptions::new().create_new(true).write(true).open(&path) {
				Ok(mut file) => {
					file.write_all(text.as_bytes()).and_then(|_| file.sync_all())
						.map_err(|error| format!("导出文本未完整保存：{error}"))?;
					return Ok(true);
				},
				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
					next_name = next_text_export_name(&path, attempt + 2);
					next_directory = path.parent().map(std::path::Path::to_path_buf);
					app.dialog().message("所选名称已存在，旧文件未修改。请在接下来的保存窗口选择新名称，或取消导出。")
						.title("请另存导出文件").blocking_show();
				},
				Err(error) => return Err(format!("不能新建导出文件：{error}")),
			}
		}
		Err("多次选择的导出名称均已存在；未覆盖任何旧文件，请使用新的文件名重试".into())
	}).await
}
