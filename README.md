# CelesVault

Windows 教学资料收纳与归档工具，范围限定为 WPS / WPS 图片打开的 Word、PPT、Excel、PDF 与图片。

**当前进度：阶段 2 最小真实收纳/目录/导出代码已接入，Native 编译与真实行为尚未验证，不能直接投入课堂使用。**

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
- **以上Native功能未运行编译或真实测试，不代表稳定验收通过。**

监测规则保存到选定data，首次目录选择已接Native；方向与缩放仍只应用到本次会话。完整课表、固定夹、库、拖拽和安全弹出后续接入，不用localStorage冒充正式存储。详见 [阶段2说明与验收](docs/stage-2.md) 和 [图标说明](docs/branding.md)。

## 文档

- [需求基线](docs/requirements.md)
- [阶段计划](docs/implementation-plan.md)
- [架构](docs/architecture.md)
- [组件来源](docs/component-provenance.md)
- [验证与用户操作](docs/verification.md)
- [Git 迁移](docs/history/migration.md)
- [诊断工具](tools/wps-probe/README.md)

## 许可

应用及迁移的 UI 采用 GPL-3.0-only，原来源/版权/许可保留。Inter 3.19 字体采用 SIL OFL；独立诊断工具保留其 MIT 声明。应用品牌图标是用户提供的 CelesVault 图稿，资产来源见图标清单。分发安装包/EXE 时应同时满足对应源码与第三方许可义务。当前没有远程发布或 GitHub Actions 构建。
