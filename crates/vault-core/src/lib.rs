use serde::Serialize;

mod api;
pub use api::{ArchiveItem, CaptureSettings, EngineStatus, SourceRule};
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
