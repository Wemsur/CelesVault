# CelesVault 图标

2026-11-15 用户提供 `CelesVault.svg`，替换原 Celestial 占位图标。左上角只保留文字 CelesVault；原通用 UI 图标仍按 Celestial 来源使用。

- `assets/branding/CelesVault.original.svg`：原图稿，SHA256 b303fa962659eb251633ffe0c86c0fa8c9db3545b6b8f833aa1b48024d8450b3，未修改用户下载目录原文件。
- `assets/branding/CelesVault.svg`：应用派生版本，移除 Figma 画板背景，沿原圆角轮廓限制滤镜溢出，以原主体居中加安全透明边距；三维 foreignObject、色彩、纹理保持。
- `assets/branding/CelesVault-1024.png`：完整 Chromium 渲染的 1024 RGBA 主图。
- `src-tauri/icons/`：16/24/32/48/64/128/256/512/1024 PNG、128@2x、32px tray/灰色暂停/红色故障/蓝色检测/绿色归档成功、多尺寸 ICO（16–256）。
- `public/`：favicon.ico、32px favicon、192/512图标。
- Tauri bundle 图标用于 EXE、任务栏和 NSIS 快捷方式；ICO首帧为256px，其余128/64/48/32/24/16帧完整保留。托盘单独使用tray.png。

原SVG含Figma三维CSS，不能直接用不支持foreignObject的普通SVG渲染器替代。可重复资产生成：`scripts/generate-icons.py`，只生成图像，不构建应用。文件摘要见 `assets/branding/manifest.json`。没有生成macOS/Android/iOS图标，因为不在产品平台范围内。

已核对 PNG尺寸/RGBA透明通道、ICO七个尺寸、浏览器左上角图片数为0与新favicon引用。用户dev反馈任务栏模糊后，只读核对tauri-codegen2.5.4：它解码ICO第一帧，不自动选最大帧；原首帧16px被放大是根因。已调整ICO目录顺序为256/128/64/48/32/24/16，不重采样、不删帧，各图像载荷SHA256完全不变；生成脚本同步以防再次生成回退。任务栏修正效果仍需用户重启dev确认，NSIS图标未构建验证，不修改Windows图标缓存设置。
