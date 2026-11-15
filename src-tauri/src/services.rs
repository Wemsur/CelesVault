use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{Emitter, Manager};
use vault_core::VaultEngine;
use crate::fs_jobs::FsJobs;

pub struct Services {
	pub engine: Arc<VaultEngine>,
	pub files: Arc<FsJobs>,
	pub startup_error: Mutex<Option<String>>,
	pub mutation: Mutex<()>,
	pub stopping: AtomicBool,
	pub exit_ready: AtomicBool,
}

impl Services {
	pub fn new() -> Self {
		Self { engine: VaultEngine::new(), files: FsJobs::new(), startup_error: Mutex::new(None), mutation: Mutex::new(()), stopping: AtomicBool::new(false), exit_ready: AtomicBool::new(false) }
	}
	pub fn shutdown(&self) { self.stopping.store(true, Ordering::Relaxed); self.files.shutdown(); self.engine.shutdown(); }
}

pub fn start_status_updates(app: tauri::AppHandle) {
	std::thread::spawn(move || {
	let mut previous_icon = String::new();
	loop {
		let services = app.state::<Services>();
		if services.stopping.load(Ordering::Relaxed) { break; }
		if let Ok(status) = services.engine.status() {
			let _ = app.emit("vault:status", &status);
			if let Some(tray) = app.tray_by_id("celesvault") {
				let label = match status.monitor_state.as_str() { "running" => "正在监测", "paused" => "已暂停", "unconfigured" => "未配置持久目录", _ => "监测异常" };
				let _ = tray.set_tooltip(Some(format!("CelesVault · {label}")));
				let icon_state = if status.monitor_state == "error" || status.failed_count > 0 || status.has_errors || services.startup_error.lock().map(|error| error.is_some()).unwrap_or(true) || services.files.jobs().map(|jobs| jobs.iter().any(|job| job.state == "failed")).unwrap_or(true) { "error" } else if status.monitor_state == "paused" || status.monitor_state == "unconfigured" { "paused" } else { "normal" };
				if previous_icon != icon_state {
					let bytes: &[u8] = match icon_state { "error" => include_bytes!("../icons/tray-error.png"), "paused" => include_bytes!("../icons/tray-paused.png"), _ => include_bytes!("../icons/tray.png") };
					if let Ok(icon) = tauri::image::Image::from_bytes(bytes) { let _ = tray.set_icon(Some(icon)); }
					previous_icon = icon_state.into();
				}
				let controls = app.state::<crate::tray::TrayControls>();
				let pause = &controls.pause;
				let _ = pause.set_enabled(status.monitor_state == "running" || status.monitor_state == "paused");
				let _ = pause.set_text(if status.monitor_state == "paused" { "恢复监测" } else { "暂停监测" });
			}
		}
		drop(services);
		std::thread::sleep(std::time::Duration::from_secs(2));
	}});
}
