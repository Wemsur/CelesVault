use tauri::{image::Image, menu::{Menu, MenuItem}, tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent}, Manager};

pub struct TrayControls { pub pause: MenuItem<tauri::Wry> }
pub fn install(app: &tauri::App) -> tauri::Result<()> {
	let open = MenuItem::with_id(app, "open", "打开 CelesVault", true, None::<&str>)?;
	let pause = MenuItem::with_id(app, "pause", "暂停监测", false, None::<&str>)?;
	let exit = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?;
	let menu = Menu::with_items(app, &[&open, &pause, &exit])?;
	app.manage(TrayControls { pause });
	TrayIconBuilder::with_id("celesvault")
		.icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
		.tooltip("CelesVault · 等待配置持久目录")
		.menu(&menu)
		.show_menu_on_left_click(false)
		.on_menu_event(|app, event| match event.id.as_ref() {
			"open" => { if let Err(error) = crate::commands::show_main(app) { eprintln!("无法显示主窗口：{error}"); } },
			"pause" => {
				let engine = app.state::<crate::services::Services>().engine.clone();
				std::thread::spawn(move || { if let Ok(status) = engine.status() { let _ = engine.pause(status.monitor_state != "paused"); } });
			},
			"exit" => app.exit(0),
			_ => {},
		})
		.on_tray_icon_event(|tray, event| {
			if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
				if let Err(error) = crate::commands::show_main(tray.app_handle()) { eprintln!("无法显示主窗口：{error}"); }
			}
		})
		.build(app)?;
	Ok(())
}
