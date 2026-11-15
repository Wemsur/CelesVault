mod commands;
mod permissions;
mod tray;
mod services;
mod locator;
mod vault_commands;
mod fs_jobs;
mod path_guard;

use tauri::{Emitter, Manager};

pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_single_instance::init(|app, _, _| {
			if let Err(error) = commands::show_main(app) { eprintln!("无法显示现有主窗口：{error}"); }
		}))
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_opener::init())
		.manage(services::Services::new())
		.invoke_handler(tauri::generate_handler![
			commands::runtime_info,
			commands::window_state,
			commands::minimize_window,
			commands::toggle_maximize_window,
			commands::hide_window,
			vault_commands::vault_status, vault_commands::choose_data_root, vault_commands::vault_today,
			vault_commands::vault_pause, vault_commands::vault_retry, vault_commands::capture_settings,
			vault_commands::set_capture_settings, vault_commands::open_archive, vault_commands::list_drives,
			vault_commands::browse_directory, vault_commands::export_archives, vault_commands::copy_jobs,
			vault_commands::cancel_copy,
		])
		.setup(|app| {
			if !permissions::elevated().map_err(std::io::Error::other)? {
				return Err(std::io::Error::other("请以管理员身份启动 CelesVault；未启动监测。").into());
			}
			match locator::load() {
				Ok(Some(root)) => {
					if let Err(error) = app.state::<services::Services>().engine.configure_root(root) {
						*app.state::<services::Services>().startup_error.lock().map_err(|_| std::io::Error::other("状态锁不可用"))? = Some(error);
					}
				},
				Ok(None) => {},
				Err(error) => { *app.state::<services::Services>().startup_error.lock().map_err(|_| std::io::Error::other("状态锁不可用"))? = Some(error); },
			}
			tray::install(app)?;
			services::start_status_updates(app.handle().clone());
			if let Some(window) = app.get_webview_window("main") {
				let handle = window.clone();
				window.on_window_event(move |event| match event {
					tauri::WindowEvent::CloseRequested { api, .. } => {
						api.prevent_close();
						let _ = handle.hide();
						commands::emit_state(&handle);
						let _ = handle.emit("vault:was-hidden", ());
					},
					tauri::WindowEvent::Resized(_) => commands::emit_state(&handle),
					_ => {},
				});
			}
			Ok(())
		})
		.build(tauri::generate_context!())
		.expect("CelesVault 启动失败")
		.run(|app, event| {
			if let tauri::RunEvent::ExitRequested { api, .. } = event {
				let services = app.state::<services::Services>();
				if !services.exit_ready.load(std::sync::atomic::Ordering::Acquire) {
					api.prevent_exit();
					if services.stopping.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).is_ok() {
						let handle = app.clone();
						std::thread::spawn(move || {
							let services = handle.state::<services::Services>();
							services.shutdown();
							services.exit_ready.store(true, std::sync::atomic::Ordering::Release);
							drop(services);
							handle.exit(0);
						});
					}
				}
			}
		});
}
