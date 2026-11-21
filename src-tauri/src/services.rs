use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use vault_core::{CaptureActivity, CaptureActivityKind, VaultEngine};
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
	pub fn shutdown(&self) { self.stopping.store(true, Ordering::Release); self.files.shutdown(); self.engine.shutdown(); }
}

const TRAY_ACTIVITY_DURATION: Duration = Duration::from_secs(10);

// 累计归档/复制失败与健康标记只作警告；当前错误/暂停优先于短暂活动色。
fn tray_presentation(monitor_state: &str, current_errors: &[String], warnings: &[String], activity: Option<CaptureActivity>, now: Instant) -> (&'static str, String) {
	let active = activity.filter(|event| now.checked_duration_since(event.occurred_at).is_some_and(|age| age < TRAY_ACTIVITY_DURATION));
	let (icon_state, label) = if !current_errors.is_empty() { ("error", "当前错误") }
		else { match monitor_state {
			"running" => match active.map(|event| event.kind) {
				Some(CaptureActivityKind::Detected) => ("capturing", "已检测到有效捕获，正在归档"),
				Some(CaptureActivityKind::Archived) => ("success", "归档成功"),
				None => ("normal", "正在监测"),
			},
			"paused" => ("paused", "已暂停"), "unconfigured" => ("paused", "未配置持久目录"), _ => ("error", "监测状态异常"),
		} };
	let mut tooltip = format!("CelesVault · {label}");
	if let Some(error) = current_errors.first() {
		let compact = error.split_whitespace().collect::<Vec<_>>().join(" ");
		let cause: String = compact.chars().take(28).collect();
		tooltip.push_str(&format!("：{cause}{}", if compact.chars().count() > 28 { "…" } else { "" }));
		if current_errors.len() > 1 { tooltip.push_str(&format!("（另{}项错误）", current_errors.len() - 1)); }
	}
	if !warnings.is_empty() { tooltip.push_str(&format!("；警告：{}", warnings.join("、"))); }
	(icon_state, tooltip)
}

pub fn start_status_updates(app: tauri::AppHandle) {
	std::thread::spawn(move || {
		let mut previous_icon = String::new();
		loop {
			let mut next_refresh = Instant::now() + Duration::from_secs(2);
			let services = app.state::<Services>();
			if services.stopping.load(Ordering::Acquire) { break; }
			let status = services.engine.status();
			if let Ok(status) = &status { let _ = app.emit("vault:status", status); }
			if let Some(tray) = app.tray_by_id("celesvault") {
				let mut current_errors = Vec::new();
				let mut warnings = Vec::new();
				let monitor_state = match &status {
					Ok(status) => {
						if !matches!(status.monitor_state.as_str(), "running" | "paused" | "unconfigured") {
							current_errors.push(format!("监测：{}", status.monitor_detail));
						}
						if status.failed_count > 0 { warnings.push(format!("历史归档失败{}", status.failed_count)); }
						if status.has_errors { warnings.push("健康/历史状态有警告".into()); }
						status.monitor_state.as_str()
					},
					Err(error) => { current_errors.push(format!("读取监测状态失败：{error}")); "error" },
				};
				match services.startup_error.lock() {
					Ok(error) => { if let Some(error) = error.as_ref() { current_errors.push(format!("启动：{error}")); } },
					Err(_) => current_errors.push("读取启动状态失败".into()),
				}
				match services.files.watch_error() {
					Ok(Some(error)) => current_errors.push(format!("目录监听：{error}")),
					Ok(None) => {},
					Err(error) => current_errors.push(format!("读取目录监听状态失败：{error}")),
				}
				match services.files.jobs() {
					Ok(jobs) => {
						let failed = jobs.iter().filter(|job| job.state == "failed").count();
						if failed > 0 { warnings.push(format!("历史复制失败{failed}")); }
					},
					Err(error) => current_errors.push(format!("读取复制任务状态失败：{error}")),
				}
				let activity: Option<CaptureActivity> = {
					#[cfg(windows)]
					{ match services.engine.capture_activity() {
						Ok(activity) => activity,
						Err(error) => { current_errors.push(format!("读取捕获活动失败：{error}")); None },
					} }
					#[cfg(not(windows))]
					{ None }
				};
				let now = Instant::now();
				if let Some(event) = activity {
					let expires = event.occurred_at + TRAY_ACTIVITY_DURATION;
					// 轮询读取不续期；临近 10 秒时提前刷新，避免再多亮一个完整轮询周期。
					if expires > now { next_refresh = next_refresh.min(expires); }
				}
				let (icon_state, tooltip) = tray_presentation(monitor_state, &current_errors, &warnings, activity, now);
				if let Err(error) = tray.set_tooltip(Some(tooltip)) { eprintln!("更新托盘提示失败：{error}"); }
				if previous_icon != icon_state {
					let bytes: &[u8] = match icon_state { "error" => include_bytes!("../icons/tray-error.png"), "paused" => include_bytes!("../icons/tray-paused.png"), "capturing" => include_bytes!("../icons/tray-capturing.png"), "success" => include_bytes!("../icons/tray-success.png"), _ => include_bytes!("../icons/tray.png") };
					match tauri::image::Image::from_bytes(bytes) {
						Ok(icon) => match tray.set_icon(Some(icon)) {
							Ok(()) => previous_icon = icon_state.into(),
							Err(error) => eprintln!("设置托盘图标失败（{icon_state}），下次重试：{error}"),
						},
						Err(error) => eprintln!("解析托盘图标失败（{icon_state}），下次重试：{error}"),
					}
				}
				let controls = app.state::<crate::tray::TrayControls>();
				let _ = controls.pause.set_enabled(monitor_state == "running" || monitor_state == "paused");
				let _ = controls.pause.set_text(if monitor_state == "paused" { "恢复监测" } else { "暂停监测" });
			}
			if let Some(window) = app.get_webview_window("main") {
				if let (Ok(false), Ok(false), Ok(size), Ok(scale), Ok(mut config)) = (window.is_maximized(), window.is_minimized(), window.inner_size(), window.scale_factor(), services.engine.config()) {
					let logical: tauri::LogicalSize<u32> = size.to_logical(scale);
					let width = logical.width.clamp(1100, 7680);
					let height = logical.height.clamp(700, 4320);
					if config.app.window_width != width || config.app.window_height != height || config.app.window_maximized {
						if let Ok(_mutation) = services.mutation.try_lock() {
							if let Ok(current) = services.engine.config() { config = current; config.app.window_width = width; config.app.window_height = height; config.app.window_maximized = false; let _ = services.engine.save_config(config); }
						}
					}
				} else if let Ok(true) = window.is_maximized() {
					if let Ok(_mutation) = services.mutation.try_lock() { if let Ok(mut config) = services.engine.config() { if !config.app.window_maximized { config.app.window_maximized = true; let _ = services.engine.save_config(config); } } }
				}
			}
			drop(services);
			std::thread::sleep(next_refresh.saturating_duration_since(Instant::now()));
		}
	});
}

#[cfg(test)]
mod tests {
	use super::{CaptureActivity, CaptureActivityKind, Duration, Instant, TRAY_ACTIVITY_DURATION};

	fn tray_presentation(state: &str, errors: &[String], warnings: &[String]) -> (&'static str, String) {
		super::tray_presentation(state, errors, warnings, None, Instant::now())
	}

	fn event(sequence: u64, kind: CaptureActivityKind, occurred_at: Instant) -> CaptureActivity {
		CaptureActivity { sequence, kind, occurred_at }
	}

	#[test]
	fn detection_and_success_expire_at_ten_seconds_without_poll_renewal() {
		let start = Instant::now();
		for (kind, expected) in [(CaptureActivityKind::Detected, "capturing"), (CaptureActivityKind::Archived, "success")] {
			let activity = Some(event(1, kind, start));
			for elapsed in [Duration::ZERO, Duration::from_secs(2), Duration::from_millis(9_999)] {
				assert_eq!(super::tray_presentation("running", &[], &[], activity, start + elapsed).0, expected);
			}
			for elapsed in [TRAY_ACTIVITY_DURATION, Duration::from_secs(30)] {
				assert_eq!(super::tray_presentation("running", &[], &[], activity, start + elapsed).0, "normal");
			}
		}
	}

	#[test]
	fn latest_event_wins_and_expired_detection_never_revives_old_success() {
		let start = Instant::now();
		let success = Some(event(2, CaptureActivityKind::Archived, start + Duration::from_secs(1)));
		// 接受和归档可在同一轮询间完成；即使没有观察到蓝色，也必须显示真实成功的绿色。
		assert_eq!(super::tray_presentation("running", &[], &[], success, start + Duration::from_secs(2)).0, "success");
		assert_eq!(super::tray_presentation("running", &[], &[], success, start + Duration::from_millis(10_999)).0, "success");
		assert_eq!(super::tray_presentation("running", &[], &[], success, start + Duration::from_secs(11)).0, "normal");
		let next_capture = Some(event(3, CaptureActivityKind::Detected, start + Duration::from_secs(2)));
		assert_eq!(super::tray_presentation("running", &[], &[], next_capture, start + Duration::from_secs(3)).0, "capturing");
		assert_eq!(super::tray_presentation("running", &[], &[], next_capture, start + Duration::from_secs(12)).0, "normal");
		let later_success = Some(event(4, CaptureActivityKind::Archived, start + Duration::from_secs(20)));
		assert_eq!(super::tray_presentation("running", &[], &[], later_success, start + Duration::from_secs(21)).0, "success");
	}

	#[test]
	fn current_errors_and_pause_override_activity_without_resetting_expiry() {
		let start = Instant::now();
		let warnings = vec!["历史归档失败3".into(), "历史复制失败2".into()];
		for kind in [CaptureActivityKind::Detected, CaptureActivityKind::Archived] {
			let activity = Some(event(1, kind, start));
			for (state, expected) in [("paused", "paused"), ("unconfigured", "paused"), ("unknown", "error")] {
				assert_eq!(super::tray_presentation(state, &[], &warnings, activity, start).0, expected);
			}
			for state in ["running", "paused", "unconfigured", "error"] {
				assert_eq!(super::tray_presentation(state, &["当前错误".into()], &warnings, activity, start).0, "error");
			}
			let (icon, tooltip) = super::tray_presentation("running", &[], &warnings, activity, start + Duration::from_secs(9));
			assert_eq!(icon, if kind == CaptureActivityKind::Detected { "capturing" } else { "success" });
			assert!(tooltip.contains("历史归档失败3") && tooltip.contains("历史复制失败2"));
			assert_eq!(super::tray_presentation("running", &[], &warnings, activity, start + TRAY_ACTIVITY_DURATION).0, "normal");
		}
	}

	#[test]
	fn historical_warnings_do_not_change_monitor_color() {
		let warnings = vec!["历史归档失败3".into(), "健康/历史状态有警告".into(), "历史复制失败2".into()];
		for (state, expected_icon, label) in [("running", "normal", "正在监测"), ("paused", "paused", "已暂停"), ("unconfigured", "paused", "未配置持久目录")] {
			let (icon, tooltip) = tray_presentation(state, &[], &warnings);
			assert_eq!(icon, expected_icon);
			assert!(tooltip.contains(label));
			assert!(tooltip.contains("警告："));
			assert!(!tooltip.contains("当前错误"));
			for warning in &warnings { assert!(tooltip.contains(warning.as_str())); }
		}
	}

	#[test]
	fn current_errors_override_monitor_state_and_recover_when_cleared() {
		for cause in ["监测：捕获线程已退出", "启动：初始化失败", "目录监听：设备已断开", "读取监测状态失败：锁不可用", "读取复制任务状态失败：锁不可用"] {
			for state in ["running", "paused", "unconfigured", "error"] {
				let (icon, tooltip) = tray_presentation(state, &[cause.into()], &["历史复制失败2".into()]);
				assert_eq!(icon, "error");
				assert!(tooltip.contains("当前错误"));
				assert!(tooltip.contains(cause));
				assert!(tooltip.contains("历史复制失败2"));
			}
		}
		assert_eq!(tray_presentation("running", &[], &["历史归档失败3".into()]).0, "normal");
		assert_eq!(tray_presentation("unknown", &[], &[]).0, "error");
	}

	#[test]
	fn error_causes_are_compact_without_hiding_warnings() {
		let errors = vec![format!("读取状态失败：\n{}", "错误".repeat(40)), "另一项当前错误".into()];
		let (_, tooltip) = tray_presentation("running", &errors, &["历史归档失败3".into()]);
		assert!(!tooltip.contains('\n'));
		assert!(tooltip.contains('…'));
		assert!(tooltip.contains("另1项错误"));
		assert!(tooltip.ends_with("警告：历史归档失败3"));
	}
}
