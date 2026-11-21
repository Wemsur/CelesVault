# 第三方许可与来源

- **Celestial Launcher 通用 UI 与必要工具/样式**：GPL-3.0-only。复制自用户本机工作树，不依赖另一个项目运行；源 HEAD、源文件 SHA256 与必要适配见 `docs/component-provenance.md` 及 vendor provenance 清单。
- **CelesVault 应用图标**：用户于2026-11-15提供自己的图稿 `CelesVault.svg`。原稿保留于 `assets/branding/CelesVault.original.svg`；按原比例渲染完整 Figma 三维图层、透明补边，PNG/ICO 来源与摘要见同目录 manifest。应用/托盘/安装器/浏览器使用新图标，左上角仅文字。vendor 中保留的 Celestial 历史占位资产不再作为应用品牌使用。
- **Inter 3.19**：Rasmus Andersson，SIL Open Font License 1.1。五字重原 WOFF2 下载自 `https://cdn.modrinth.com/fonts/inter/Inter-{Regular,Medium,SemiBold,Bold,ExtraBold}.woff2?v=3.19`，与原组件字体版本一致。许可取自 `https://raw.githubusercontent.com/rsms/inter/v3.19/LICENSE.txt`，完整文本位于 `public/fonts/inter/OFL.txt`。
- **运行依赖**：Vue、Vue Router、Pinia、VueUse、Floating Vue、OverlayScrollbars、marked、DOMPurify、Tauri；具体锁定版本及传递依赖见 `pnpm-lock.yaml`。构建/分发时保留各自许可证，不用应用 GPL 声明替换第三方许可。
- **bilibili-margin 背景源码**：MIT，Copyright (c) 2026 jiuyang5354。本地源路径、摘要、CelesVault改动与原上游参考保存在 `src/vendor/margin-background/provenance.json`，原完整MIT在同目录LICENSE。Builder系列原注释标明21st.dev Shader Builder；弥散/Mesh Gradient改编自Paper Shaders（Apache-2.0），保留相关说明与Apache-2.0全文（LICENSE-APACHE）。汇流保留Gateway Flow/Neuform来源；未引入WebGPU/vgpu或参考项目字体/网页业务。
- **独立诊断工具**：`tools/wps-probe` 保留原 MIT 声明；迁移没有重写原历史许可。

分发 CelesVault 二进制时提供适用 GPL 的完整对应源码，包括必要构建配置、依赖清单和本文件引用的许可；源码包不能只有新业务文件而遗漏 vendor。未在本轮推送或公开发布。
