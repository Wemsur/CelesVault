use serde::Serialize;

mod api;
pub use api::*;
mod config;
mod library;
mod timetable;
pub use config::{parse_config, validate_app, validate_capture, validate_config, validate_favorites};
pub use library::validate_library_query;
pub use timetable::{classify_course, export_timetable, import_timetable, parse_date, time_minutes, validate_timetable};
#[cfg(windows)]
mod archive;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod secure_fs;
#[cfg(windows)]
mod storage;
#[cfg(windows)]
mod schema;
#[cfg(windows)]
mod activity;
#[cfg(windows)]
mod recovery;
#[cfg(windows)]
mod thumbnail;
#[cfg(windows)]
pub use engine::VaultEngine;
#[cfg(windows)]
pub use archive::ArchiveLease;
#[cfg(not(windows))]
mod unavailable;
#[cfg(not(windows))]
pub use unavailable::{ArchiveLease,VaultEngine};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
	pub capture: bool,
	pub archive: bool,
	pub file_browser: bool,
	pub copy: bool,
	pub eject: bool,
	pub persistent_settings: bool,
}

impl Default for Capabilities {
	fn default() -> Self {
		Self {
			capture: false,
			archive: false,
			file_browser: false,
			copy: false,
			eject: false,
			persistent_settings: false,
		}
	}
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
	pub mode: &'static str,
	pub version: String,
	pub elevated: bool,
	pub default_data_root: String,
	pub monitor_state: &'static str,
	pub monitor_detail: &'static str,
	pub capabilities: Capabilities,
}
