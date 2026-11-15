use tauri::{Emitter, Manager, WebviewWindow};
use vault_core::{Capabilities, RuntimeInfo};

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
	maximized: bool,
	visible: bool,
}

#[tauri::command]
pub fn window_state(window: WebviewWindow) -> Result<WindowState, String> {
	Ok(WindowState {
		maximized: window.is_maximized().map_err(|e| e.to_string())?,
		visible: window.is_visible().map_err(|e| e.to_string())?,
	})
}

pub fn emit_state(window: &WebviewWindow) {
	if let Ok(state) = window_state(window.clone()) {
		let _ = window.emit("vault:window-state", state);
	}
}

#[tauri::command]
pub fn minimize_window(window: WebviewWindow) -> Result<(), String> {
	window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_maximize_window(window: WebviewWindow) -> Result<(), String> {
	if window.is_maximized().map_err(|e| e.to_string())? {
		window.unmaximize().map_err(|e| e.to_string())?;
	} else {
		window.maximize().map_err(|e| e.to_string())?;
	}
	emit_state(&window);
	Ok(())
}

#[tauri::command]
pub fn hide_window(window: WebviewWindow) -> Result<(), String> {
	window.hide().map_err(|e| e.to_string())?;
	emit_state(&window);
	Ok(())
}

pub fn show_main(app: &tauri::AppHandle) -> tauri::Result<()> {
	if let Some(window) = app.get_webview_window("main") {
		window.show()?;
		window.unminimize()?;
		window.set_focus()?;
		emit_state(&window);
	}
	Ok(())
}

#[tauri::command]
pub fn runtime_info(app: tauri::AppHandle) -> Result<RuntimeInfo, String> {
	let executable = std::env::current_exe().map_err(|e| e.to_string())?;
	let parent = executable.parent().ok_or("无法定位程序目录")?;
	let status = app.state::<crate::services::Services>().engine.status()?;
	Ok(RuntimeInfo {
		mode: "native",
		version: app.package_info().version.to_string(),
		elevated: crate::permissions::elevated()?,
		default_data_root: parent.join("data").to_string_lossy().into_owned(),
		monitor_state: if status.monitor_state == "running" { "running" } else if status.monitor_state == "paused" { "paused" } else if status.monitor_state == "error" { "error" } else { "unavailable" },
		monitor_detail: "当前为阶段2最小收纳闭环；完整课表与固定夹尚未接入。",
		capabilities: Capabilities { capture: true, archive: true, file_browser: true, copy: true, eject: false, persistent_settings: true },
	})
}
