# CelesVault

Windows 教学资料收纳与归档工具，范围限定为 WPS / WPS 图片打开的 Word、PPT、Excel、PDF 与图片。

**当前进度：用户已通过本机 Native dev 编译并启动。持久目录初始化的问题已修正，真实监测/归档/导出仍待复测，不能直接投入课堂使用。**

## 开发

Node.js 22.12+ 或 24，pnpm 10.15。前端依赖已在当前工作目录安装；换电脑先安装：

```bash
pnpm install --ignore-scripts
```

只预览前端（开发浏览器模式明确标注，不访问真实文件）：

```bash
pnpm run web:dev
```

完整 Windows/Tauri 开发入口，**由用户在主动打开的管理员终端中运行**：

```bash
pnpm run app:dev
```

需要 Rust MSVC 工具链、Windows SDK/MSVC 和 WebView2。manifest 请求管理员权限。若 Vite 已由 Browser pane 使用 8006 端口，请先停掉仅前端预览，再运行 app:dev；`--strictPort` 不会悄悄换端口导致 Tauri 指错页面。不要同时运行两个前端 dev 服务。

前端主页 `http://localhost:8006/`；库为 `/#/library`；原控件展示为 `/#/__components`，只在开发模式可用。正式静态页面无 Tauri 宿主时拒绝启动，不静默显示假数据。

## 当前交付

- 独立 Git 历史、原控件、中文主题与无外边距两半主页。
- 用户自己的 CelesVault 图标全部规格（含多尺寸ICO和托盘状态），左上角仅文字。
- Native持久目录选择、SQLite意图/归档/每日未分类/40分钟重试代码。
- 真实目录列表、资料选择/导出/取消，按源/目标句柄身份防误写；只支持保留两份/跳过，明确替换后续接入。
- 窗口/托盘/管理员代码、原设置与开发控件展示。
- **用户已确认Native dev编译启动成功；真实目录初始化、监测/归档/复制仍待复测，不代表稳定验收通过。**

监测规则保存到选定data，首次目录选择已接Native；方向与缩放仍只应用到本次会话。完整课表、固定夹、库、拖拽和安全弹出后续接入，不用localStorage冒充正式存储。详见 [阶段2说明与验收](docs/stage-2.md) 和 [图标说明](docs/branding.md)。

## 高优先级：科目排除与监测重启

源码已接入“设置 → 监测 → 排除科目”和“清理并重启监测”。排除按课表归属与稳定科目 ID 判断，保存时停止旧后台再应用，保留已完成历史；不是按内容识别学科。用户确认清理重启时，会强制结束其他所有同名CelesVault/Probe进程并清理CelesVault的ETW残留，保留当前窗口；普通开发启动器仍只请求已登记旧实例正常退出。

这些新修改尚未通过 Windows 编译或 Native 验收；旧版 dev 启动成功不代表当前版本已通过。见 [行为与验收清单](docs/subject-exclusions.md)、[开发残留边界](docs/dev-lifecycle.md)。

## Windows 构建与草稿发布

`.github/workflows/windows-build.yml` 仅手动触发，构建 Windows x64 便携EXE和NSIS，并创建未公开的 GitHub Release 草稿。Tag 自动读取根 `package.json` 的版本（当前 `v0.2.0`），校验 Cargo/Tauri版本一致；不覆盖已有版本。附带SHA-256、构建提交、对应源码和许可。

当前仅配置完成，未提交/推送/运行 CI 或创建 Release。两个锁文件均须纳入仓库；详细操作、重复版本与学校验收边界见 [构建与部署](docs/deployment.md)。

## 文档

- [需求基线](docs/requirements.md)
- [阶段计划](docs/implementation-plan.md)
- [架构](docs/architecture.md)
- [组件来源](docs/component-provenance.md)
- [验证与用户操作](docs/verification.md)
- [Git 迁移](docs/history/migration.md)
- [诊断工具](tools/wps-probe/README.md)

## 许可

应用及迁移的 UI 采用 GPL-3.0-only，原来源/版权/许可保留。Inter 3.19 字体采用 SIL OFL；独立诊断工具保留其 MIT 声明。应用品牌图标是用户提供的 CelesVault 图稿，资产来源见图标清单。分发安装包/EXE 时应同时满足对应源码与第三方许可义务。当前已编写手动 Windows x64 构建与 GitHub Release 草稿工作流，但尚未运行或远程发布。
