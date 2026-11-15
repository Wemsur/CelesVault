fn main() {
	let windows = tauri_build::WindowsAttributes::new()
		.app_manifest(include_str!("app.manifest"));
	tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
		.expect("无法生成 CelesVault 的 Tauri 构建资源");
}
