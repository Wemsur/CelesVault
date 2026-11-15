#[cfg(windows)]
pub fn elevated() -> Result<bool, String> {
	use std::{mem::size_of, ptr::null_mut};
	use windows_sys::Win32::{
		Foundation::{CloseHandle, HANDLE},
		Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
		System::Threading::{GetCurrentProcess, OpenProcessToken},
	};

	unsafe {
		let mut token: HANDLE = null_mut();
		if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
			return Err(std::io::Error::last_os_error().to_string());
		}
		let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
		let mut returned = 0;
		let result = GetTokenInformation(token, TokenElevation,
			&mut elevation as *mut _ as *mut _, size_of::<TOKEN_ELEVATION>() as u32, &mut returned);
		let error = if result == 0 { Some(std::io::Error::last_os_error().to_string()) } else { None };
		CloseHandle(token);
		if let Some(error) = error { return Err(error); }
		Ok(elevation.TokenIsElevated != 0)
	}
}

#[cfg(not(windows))]
pub fn elevated() -> Result<bool, String> {
	Err("CelesVault 第一版只支持 Windows".into())
}
