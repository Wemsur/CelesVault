mod commands;
mod permissions;
mod tray;
mod services;
mod locator;
mod vault_commands;
mod fs_jobs;
mod path_guard;
mod devices;
mod monitoring_cleanup;
#[cfg(all(windows, debug_assertions))]
mod dev_lifecycle;

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
			vault_commands::vault_pause, vault_commands::restart_monitoring, vault_commands::vault_retry, vault_commands::capture_settings,
			vault_commands::set_capture_settings, vault_commands::open_archive, vault_commands::list_drives,
			vault_commands::browse_directory, vault_commands::export_archives, vault_commands::copy_jobs,
			vault_commands::cancel_copy,
			vault_commands::vault_config, vault_commands::save_vault_config, vault_commands::save_app_settings,
			vault_commands::vault_courses, vault_commands::change_course, vault_commands::library_query,
			vault_commands::archive_detail, vault_commands::edit_archive, vault_commands::move_usage,
			vault_commands::save_favorite, vault_commands::remove_favorite, vault_commands::trash_archive,
			vault_commands::trash_list, vault_commands::restore_archive, vault_commands::purge_archive,
			vault_commands::archive_thumbnail, vault_commands::import_timetable, vault_commands::import_config,
			vault_commands::export_timetable, vault_commands::export_config, vault_commands::preview_copy,
			vault_commands::start_copy_plan, vault_commands::create_folder, vault_commands::rename_entry,
			vault_commands::recycle_entry, vault_commands::open_browser_entry, vault_commands::reveal_browser_entry,
			vault_commands::watch_directory, vault_commands::unwatch_directory, vault_commands::file_recoveries,
			vault_commands::restore_file_recovery, vault_commands::eject_device, vault_commands::save_text_export,
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
				if let Ok(config) = app.state::<services::Services>().engine.config() {
					window.set_size(tauri::LogicalSize::new(config.app.window_width, config.app.window_height))?;
					if config.app.window_maximized { window.maximize()?; }
				}
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
			#[cfg(all(windows, debug_assertions))]
			dev_lifecycle::start(app.handle().clone());
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
