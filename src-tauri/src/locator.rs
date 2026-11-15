use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Locator { version: u32, data_root: String }

fn location() -> Result<PathBuf, String> {
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	Ok(exe.parent().ok_or("程序目录不可用")?.join("celesvault-location.json"))
}

pub fn load() -> Result<Option<String>, String> {
	let path = location()?;
	if !path.exists() { return Ok(None); }
	reject_link(&path)?;
	let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
	if bytes.len() > 8192 { return Err("持久目录定位文件过大".into()); }
	let locator: Locator = serde_json::from_slice(&bytes).map_err(|e| format!("持久目录定位文件损坏：{e}"))?;
	if locator.version != 1 { return Err("持久目录定位版本不受支持".into()); }
	Ok(Some(locator.data_root))
}

pub fn save(data_root: &str) -> Result<(), String> {
	use std::io::Write;
	let path = location()?;
	if path.exists() { reject_link(&path)?; }
	let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
	let bytes = serde_json::to_vec_pretty(&Locator { version: 1, data_root: data_root.into() }).map_err(|e| e.to_string())?;
	let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&temporary).map_err(|e| e.to_string())?;
	if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
		drop(file); let _ = std::fs::remove_file(&temporary); return Err(error.to_string());
	}
	drop(file);
	#[cfg(windows)] {
		use std::os::windows::ffi::OsStrExt;
		use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
		let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
		let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
		if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
			let error = std::io::Error::last_os_error().to_string(); let _ = std::fs::remove_file(temporary); return Err(error);
		}
	}
	#[cfg(not(windows))] std::fs::rename(temporary, path).map_err(|e| e.to_string())?;
	Ok(())
}

fn reject_link(path: &Path) -> Result<(), String> {
	let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
	if metadata.file_type().is_symlink() { return Err("定位文件不可是链接".into()); }
	#[cfg(windows)] {
		use std::os::windows::fs::MetadataExt;
		if metadata.file_attributes() & 0x400 != 0 { return Err("定位文件不可是重解析点".into()); }
	}
	Ok(())
}
