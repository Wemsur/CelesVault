//! 仅由 Windows debug 应用的 setup 启用；不提供前端命令或窗口消息入口。
#![cfg(all(windows, debug_assertions))]

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
	fs::{self, File, OpenOptions},
	io::{self, Read, Write},
	os::windows::fs::{MetadataExt, OpenOptionsExt},
	path::Path,
	thread,
	time::Duration,
};
use windows_sys::Win32::{
	Foundation::FILETIME,
	Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT},
	System::Threading::{GetCurrentProcess, GetProcessTimes},
};

#[derive(Serialize)]
struct Registration<'a> {
	pid: u32,
	created: String,
	exe: &'a str,
	proof: String,
}

/// 忽略没有包装器环境的普通 debug 启动；握手失败仅报告，不认领该进程。
pub(crate) fn start(app: tauri::AppHandle) {
	let (Ok(session), Ok(token)) = (
		std::env::var("CELESVAULT_DEV_SESSION"),
		std::env::var("CELESVAULT_DEV_TOKEN"),
	) else { return };
	if let Err(error) = register(app, &session, &token) {
		eprintln!("未登记 CelesVault 开发实例（PID {}）：{error}。需要时请从对应托盘手动退出。", std::process::id());
	}
}

fn register(app: tauri::AppHandle, session: &str, token: &str) -> io::Result<()> {
	if !is_hex(session, 32) || !is_hex(token, 64) {
		return Err(io::Error::other("开发握手环境格式无效"));
	}
	let project = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent().ok_or_else(|| io::Error::other("无法确定项目目录"))?.canonicalize()?;
	let target = project.join("target");
	let debug = target.join("debug");
	let store = target.join("celesvault-dev");
	let dir = store.join(session);
	for path in [&target, &debug, &store, &dir] {
		let meta = fs::symlink_metadata(path)?;
		if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
			return Err(io::Error::other("拒绝使用非目录或重解析点作为开发握手路径"));
		}
	}
	let expected = debug.join("celesvault.exe");
	let meta = fs::symlink_metadata(&expected)?;
	if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		return Err(io::Error::other("开发 EXE 不是普通文件"));
	}
	let exe = std::env::current_exe()?.canonicalize()?;
	if exe != expected.canonicalize()? {
		return Err(io::Error::other("当前 EXE 不属于项目的默认 debug 目标"));
	}
	let exe = exe.to_str().ok_or_else(|| io::Error::other("无法表示开发 EXE 路径"))?;
	if !matches!(read_fixed(&dir.join(".env.celesvault-ticket")), Ok(bytes) if bytes.as_slice() == token.as_bytes()) {
		return Err(io::Error::other("开发票据与本进程的启动令牌不匹配"));
	}

	let pid = std::process::id();
	let created = creation_filetime()?.to_string();
	let identity = format!("{session}\n{pid}\n{created}\n{exe}");
	let registration = Registration {
		pid, created: created.clone(), exe, proof: proof(token, "register-v1", &identity),
	};
	let request_proof = proof(token, "exit-v1", &identity);
	let base = format!("{pid}-{created}");
	let record = dir.join(format!("{base}.json"));
	let pending = dir.join(format!("{base}.tmp"));
	let request = dir.join(format!("{base}.exit"));
	let bytes = serde_json::to_vec(&registration).map_err(io::Error::other)?;
	let mut file = OpenOptions::new().write(true).create_new(true).open(&pending)?;
	let written = file.write_all(&bytes);
	drop(file);
	if let Err(error) = written.and_then(|()| fs::rename(&pending, &record)) {
		let _ = fs::remove_file(&pending);
		return Err(error);
	}
	let result = thread::Builder::new().name("dev-lifecycle".into()).spawn(move || {
		loop {
			thread::sleep(Duration::from_millis(250));
			if matches!(read_fixed(&request), Ok(bytes) if bytes.as_slice() == request_proof.as_bytes()) {
				// 由现有 RunEvent::ExitRequested 负责 shutdown；绝不通过关闭窗口退出。
				let handle = app.clone();
				if let Err(error) = app.run_on_main_thread(move || handle.exit(0)) {
					eprintln!("无法分派开发实例的退出请求：{error}");
				}
				return;
			}
		}
	});
	if let Err(error) = result {
		let _ = fs::remove_file(&record);
		return Err(error);
	}
	Ok(())
}

fn read_fixed(path: &Path) -> io::Result<[u8; 64]> {
	let file: File = OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
	let info = file.metadata()?;
	if !info.is_file() || info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.len() != 64 {
		return Err(io::Error::other("开发握手文件无效"));
	}
	let mut bytes = [0; 64];
	file.take(64).read_exact(&mut bytes)?;
	Ok(bytes)
}

fn is_hex(value: &str, size: usize) -> bool {
	value.len() == size && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn creation_filetime() -> io::Result<u64> {
	const ZERO: FILETIME = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
	let (mut creation, mut exit, mut kernel, mut user) = (ZERO, ZERO, ZERO, ZERO);
	// 伪句柄只指向自身，不需要关闭。
	if unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
		return Err(io::Error::last_os_error());
	}
	Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

// HMAC-SHA256，令牌恰好是 64 字节 ASCII（SHA256 的一个块）。与 Node 使用同样的字节。
fn proof(token: &str, kind: &str, identity: &str) -> String {
	let (mut inner_pad, mut outer_pad) = ([0x36; 64], [0x5c; 64]);
	for (idx, byte) in token.bytes().enumerate() {
		inner_pad[idx] ^= byte;
		outer_pad[idx] ^= byte;
	}
	let mut inner = Sha256::new();
	inner.update(inner_pad);
	inner.update(kind.as_bytes());
	inner.update(b"\n");
	inner.update(identity.as_bytes());
	let mut outer = Sha256::new();
	outer.update(outer_pad);
	outer.update(inner.finalize());
	format!("{:x}", outer.finalize())
}
