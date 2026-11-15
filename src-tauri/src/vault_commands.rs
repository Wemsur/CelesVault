use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use vault_core::{ArchiveItem, CaptureSettings, EngineStatus};
use crate::{services::Services, fs_jobs::{CopyJob, CopyRequest, DirectoryView, DriveInfo, ExportSource}};

#[tauri::command]
pub async fn vault_status(services: State<'_, Services>) -> Result<EngineStatus, String> {
	let engine = services.engine.clone();
	let mut status = tauri::async_runtime::spawn_blocking(move || engine.status()).await.map_err(|e| e.to_string())??;
	if let Some(error) = services.startup_error.lock().map_err(|_| "状态锁不可用")?.clone() { status.health.push(error); status.has_errors = true; }
	Ok(status)
}

#[tauri::command]
pub async fn choose_data_root(app: tauri::AppHandle, services: State<'_, Services>) -> Result<Option<EngineStatus>, String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || {
		let executable = std::env::current_exe().map_err(|e| e.to_string())?;
		let default = executable.parent().ok_or("无法读取程序目录")?.join("data");
		let selected = app.dialog().file().set_title("选择不受还原影响的持久目录（默认程序同级 data）").set_directory(&default).blocking_pick_folder();
		let Some(selected) = selected else { return Ok(None); };
		let path = selected.into_path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
		let service = app.state::<Services>();
		let _mutation = service.mutation.lock().map_err(|_| "目录切换锁不可用")?;
		if service.files.jobs()?.iter().any(|job| job.state == "running") { return Err("复制任务仍在运行，不能更换持久目录".into()); }
		let status = engine.configure_root(path)?;
		if let Some(root) = &status.data_root { crate::locator::save(root).map_err(|e| format!("资料库已初始化，但持久目录定位未保存，下次需重新选择：{e}"))?; }
		*app.state::<Services>().startup_error.lock().map_err(|_| "状态锁不可用")? = None;
		Ok(Some(status))
	}).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn vault_today(services: State<'_, Services>) -> Result<Vec<ArchiveItem>, String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || engine.list_today()).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn vault_pause(services: State<'_, Services>, paused: bool) -> Result<EngineStatus, String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || engine.pause(paused)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn vault_retry(services: State<'_, Services>, task_id: String) -> Result<(), String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || engine.retry(task_id)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn capture_settings(services: State<'_, Services>) -> Result<CaptureSettings, String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || engine.get_capture_settings()).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn set_capture_settings(services: State<'_, Services>, settings: CaptureSettings) -> Result<(), String> {
	let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || engine.set_capture_settings(settings)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn open_archive(app: tauri::AppHandle, services: State<'_, Services>, id: String) -> Result<(), String> {
	let engine = services.engine.clone();
	let path = tauri::async_runtime::spawn_blocking(move || engine.archived_path(id)).await.map_err(|e| e.to_string())??;
	app.opener().open_path(path.to_string_lossy().into_owned(), None::<&str>).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn list_drives(services: State<'_, Services>) -> Result<Vec<DriveInfo>, String> {
	let files = services.files.clone();
	tauri::async_runtime::spawn_blocking(move || files.list_drives()).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn browse_directory(services: State<'_, Services>, path: String) -> Result<DirectoryView, String> {
	let files = services.files.clone(); let engine = services.engine.clone();
	tauri::async_runtime::spawn_blocking(move || files.browse(path, engine.status()?.data_root)).await.map_err(|e| e.to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest { ids: Vec<String>, target_path: String, target_identity: String, conflict: String }

#[tauri::command]
pub async fn export_archives(app: tauri::AppHandle, services: State<'_, Services>, request: ExportRequest) -> Result<CopyJob, String> {
	let engine = services.engine.clone(); let files = services.files.clone();
	tauri::async_runtime::spawn_blocking(move || {
		if request.ids.is_empty() || request.ids.len() > 1000 { return Err("请选择1–1000份已归档资料".into()); }
		let service = app.state::<Services>();
		let _mutation = service.mutation.lock().map_err(|_| "导出状态锁不可用")?;
		let records = engine.list_today()?;
		let mut leases = Vec::new();
		let mut sources = Vec::new();
		let mut ids = std::collections::HashSet::new();
		for id in request.ids {
			if !ids.insert(id.clone()) { continue; }
			let item = records.iter().find(|item| item.id == id && item.state == "available").ok_or("只能导出当天已成功保存的资料")?;
			let lease = engine.lease_archive(id.clone())?;
			sources.push(ExportSource { id: id.clone(), display_name: item.display_name.clone(), path: lease.path.clone() });
			leases.push(lease);
		}
		let job = files.start_copy(sources, CopyRequest { target_path: request.target_path, target_identity: request.target_identity, conflict: request.conflict }, engine.status()?.data_root)?;
		drop(leases);
		Ok(job)
	}).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn copy_jobs(services: State<'_, Services>) -> Result<Vec<CopyJob>, String> { services.files.jobs() }
#[tauri::command]
pub fn cancel_copy(services: State<'_, Services>, job_id: String) -> Result<(), String> { services.files.cancel(job_id) }
