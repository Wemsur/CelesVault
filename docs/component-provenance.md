# CelesVault 组件来源与修改记录

## 来源基线

- 抽取日期：2026-11-14。
- 参考工作树：`C:/Users/iexam/github-project/CelestialLauncher`，只读使用，没有修改原仓库。
- 已实际读取并核对 HEAD：`3a384015be700fc3eba3351dd4a3339bc3a9d31d`。
- source origin：`https://github.com/Wemsur/modrinth.git`；上游 `https://github.com/modrinth/code.git`。本次使用本地当前文件字节，不使用远程下载或用 HEAD 内容覆盖工作树。
- 原工作树有与当前提取范围无关的设置、翻译、下载等未提交修改；未移动、还原或修改它们。
- 依据本项目 `requirements.md`、`implementation-plan.md` 及原仓库根、packages/ui 的 `AGENTS.md`。原 App frontend、assets、tooling-config 没有额外适用 AGENTS 文件。

本次完成阶段 1 的 **vendor 原组件闭包**，不是界面重新设计或后续业务实现。组件迁移子代理只写 `src/vendor/celestial/**` 和本文档；主线程另行负责布局、业务骨架、平台、依赖、离线字体及 Tauri 宿主，并执行允许的前端预览。

## 可追溯清单

机器可读的逐文件清单：[`src/vendor/celestial/provenance/origin-manifest.json`](../src/vendor/celestial/provenance/origin-manifest.json)。每个已复制文件记录：

- 原相对路径和源 HEAD（本地第三方依赖许可文件不冒称属于该 Git HEAD）；
- **当前原文件**完整字节 SHA-256；
- 最终 vendor 文件 SHA-256；
- 许可标识与具体修改说明；
- 片段提取的源行范围，例如 Launcher global 的 light-sense。

原样拷贝的组件、SVG、许可保留字节。经过适配的文件保留原 SFC template/styles、源码注释与许可信息，另注明 CelesVault 修改日期。新精简出口、样式入口、Markdown 适配单独记录为生成/适配文件，不冒称原库有同名实现。清单不对自身计算递归摘要；日后改 vendor 文件须同步更新其摘要和修改记录。

## 抽取范围与修改

| vendor 路径 | 原路径 | 内容/修改 |
|---|---|---|
| `ui/components/launcher/NavButton.vue` | `apps/app-frontend/src/components/ui/NavButton.vue` | 原样复制。保留 RouterLink/函数导航、active/subpage class、圆形布局和导航交互 |
| `ui/components/launcher/WindowControls.vue` | 同目录 `WindowControls.vue` | 原 Teleport、DOM、视觉尺寸和扩展命中区域保留；删除原 Tauri/settings/OS 业务导入，接宿主 usePlatform；关闭变 hide-to-tray，中文标签，showControls/width-change/window-error契约 |
| `ui/components/base/{Card,Checkbox,Toggle}.vue` | `packages/ui/src/components/base/` | 原 Card/Checkbox/Toggle；Card 只改直接导入和中文 label，其余 DOM、样式、模型和 motion 保留 |
| `ui/components/base/buttons/{ButtonFrame,Button,IconButton}.vue`、`types.ts` | 原同目录 | 原样复制；原 types/size/color 与共享按钮结构不重写 |
| `ui/components/base/inputs/{Input,InputFrame,InputClearButton}.vue`、`types.ts` | 原同目录 | 原输入闭包；只将清除标签改为中文，保留 model、prefix/leading/trailing/right slots 和输入外观 |
| `ui/components/modal/{NewModal,TabbedModal,ConfirmModal}.vue` | 原同目录 | 原布局、堆栈、焦点、退出/展开/滚动渐隐保留；去掉全部翻译调用/Provider，改普通中文 strings；叶子导入；移除 NewModal 未声明的 legacy default；ConfirmModal 指向安全本地 Markdown |
| `ui/components/base/{DropdownSelect,Combobox}.vue` | 原同目录 | 原 DOM、样式、下拉定位和交互保留；中文默认占位/空态；Combobox 保留实际 OverlayScrollbars 依赖，不重造选择器 |
| `ui/components/base/buttons/{ContextMenu,TeleportOverflowMenu}.vue` | 原同目录 | 使用通用 UI 原组件，不携带 Launcher 业务包装；ContextMenu原样。主线程仅修正 TeleportOverflowMenu 的 aria-label 透传：避免未定义 attrs 覆盖图标按钮的原中文标签，DOM/视觉不变 |
| `ui/components/base/buttons/button-menu/` | 原同目录 4 个实际依赖文件 | button-menu.ts、ButtonMenuItem、ButtonMenuPanel、ButtonMenuSubmenu 原样复制；保留键盘、hover intent、submenu、tooltip、tone、trailing action |
| `ui/components/base/LoadingIndicator.vue` | 原同目录 | 原 modal suspense fallback；仅文字改“加载中” |
| `ui/composables/{modal-stack,scroll-indicator,debug-logger}.ts` | 原同目录 | modal-stack/scroll-indicator 原样复制；debug-logger 的 Node process production 判据改为 Vite import.meta.env.PROD |
| `ui/utils/{truncate,use-anchored-teleport}.ts` | 原同目录 | 原叶子工具原样复制；无需复杂浮层重新实现 |
| `ui/providers/{create-context,modal-behavior}.ts` | 原同目录 | 原 UI 叶子 DI；create-context 保留完整 UnoVue MIT notice。不复制原 provider 总 barrel |
| `ui/utils/markdown.ts` | 接口原用于 `packages/utils/parse.ts` | 新本地 marked + DOMPurify 安全适配；原网络图片代理/共享 utils **不复制**。HTML profile 禁止远程资源/嵌入/脚本/样式等；DOMPurify 不支持时只输出转义文字 |
| `ui/index.ts`、按钮/输入/provider `index.ts` | 新建精简出口 | 仅导出复制组件和所需 types/providers/composables；不加载业务/API、账号、通知、编辑器、3D、翻译或原全部模块 |
| `styles/{variables,normalize,reset,defaults,accessibility}.scss` | `packages/assets/styles/` | 原样式规则保留；defaults仅将reset的旧Sass import改为use，消除弃用警告，customdark/surface/brand/text及间距圆角不改 |
| `styles/classes.scss` | 原同名文件 | 保留 Card、Markdown、工具提示等原规则；只删除未用原 native-select 中的远程 placeholder URL |
| `styles/tailwind-utilities.scss` | `packages/ui/src/styles/tailwind-utilities.css` | 内容字节不变，扩展名改 SCSS 用于本地内联；保留浮层/菜单/heading utilities |
| `styles/tailwind-preset.ts` | `packages/tooling-config/tailwind/tailwind-preset.ts` | 保留原 token map/preflight=false；去掉闭包没有使用的 container-query plugin，避免额外无关依赖 |
| `styles/launcher-{before,after}.scss` | `apps/app-frontend/src/assets/stylesheets/global.scss` | 只提取原 body/link/scrollbar/input 与 light-sense 必需规则；保留原 cascade 顺序，源范围见清单。不带入整个 App 或背景业务样式 |
| `styles/index.scss` | 新建本地入口 | 本地依赖闭包，无远程字体或原assets barrel；2026-11-15改为sass:meta.load-css，逐项保持原CSS加载顺序，移除旧Sass import弃用警告 |
| `assets/icons/*.svg` | `packages/assets/icons/` | 精选 39 个实际 SVG 字节拷贝，保留已有 Lucide notice，完整许可另存 |
| `assets/generated-icons.ts`、`assets/index.ts` | 原 generated-icons.ts 选取导出/新精简入口 | 只生成需要的原名字/路径，不复制图标映射或所有资产 barrel，不添加伪称原有的图标名 |
| `assets/celestial.svg` | `apps/app-frontend/src/assets/celestial.svg` | 用户原 Celestial logo 字节拷贝，保留 CSS brand 色；出口名 CelestialLogo |
| `provenance/licenses/` | 根和 3 个来源 package 的 COPYING/LICENSE，已安装 Lucide 包 LICENSE | 逐字保留原 GPL、COPYING 版权/品牌条件和 Lucide/Feather ISC/MIT 许可 |

## 2026-11-20：操作通知接入

直接复制本地 Celestial 当前工作树的 `packages/ui/src/components/nav/NotificationPanel.vue` 与 `providers/web-notifications.ts`，不以自制卡片仿造通知。保留四类颜色/图标（成功、提示、警告、错误）、堆叠与进退场、悬停计时、关闭、复制文字和操作按钮。仅做中文标签、离线依赖与浏览器计时类型适配；移除无关 Intercom 观察器，不引入 i18n、账号、远程图片、系统推送或 Popup 社交/下载业务。保留现有 GPL/图标许可。

新增原 SVG `check-circle`、`issues`、`x-circle` 字节拷贝；机器清单包含当前源/目标 SHA-256。`src/platform/notifications.ts` 的本地存储实现沿用 `apps/app-frontend/src/providers/app-notifications.ts` 的原数组模式，扩展任务ID更新、最多六条普通通知保留及从持久设置读取时长。宿主 `OperationNotifications.vue` 只把真实复制任务与操作错误接到原面板，不伪造成功。进度不超时；结束只计时一次，不因轮询重新弹出。

右下角为避让安全弹出按钮，宿主仅增加面板底部偏移、视口宽高边界和溢出滚动，不改变原通知内部布局。现已不再使用旧的常驻复制结果文本块。设置提供四类显式示例（文字说明为示例）及0–120秒，默认6秒；保存后生效。

## 许可与版权

原 `packages/ui/COPYING.md`、`packages/assets/COPYING.md`、`apps/app-frontend/COPYING.md` 指定源码为 **GPL-3.0-only**，对应完整 LICENSE 和原根 COPYING 均已存入 vendor 的 `provenance/licenses/`。原 Rinth, Inc. 版权/品牌限制文字没有删改。

- UnoVue create-context 的 MIT 许可/版权保留在原源文件顶部。
- normalize.scss 保留 normalize.css v8.0.1 的原 MIT 许可来源头。
- Lucide 图标保留原 SVG 头（若有）和原完整 ISC/MIT 许可；包括 Feather/Cole Bemis 和 Lucide Contributors 的版权说明。
- 没有抽取 Modrinth logo、Rinthbot、外部品牌或原 UI 品牌组件。占位 logo 是用户提供并拥有的 Celestial 原 logo，不是 Modrinth 标识。
- 原 Inter3.19 离线文件/字体许可由主线程负责；vendor 未复制或导入原联网 inter.scss，也未下载或替换字体。
- 分发本项目时应遵守 GPL 的许可文本、修改标识和相应源码提供义务；不能仅凭本 vendor 清单宣称整个发行包已完成许可验收。

## 接入与依赖

完整的别名、正确样式入口、初始化、props、model、slot、Modal show/hide 与菜单调用示例见 [`provenance/integration.md`](../src/vendor/celestial/provenance/integration.md)。关键项：

- `@modrinth/ui` → `src/vendor/celestial/ui/index.ts`；`@modrinth/assets` → `assets/index.ts`；`#ui` → vendor ui 目录；`@` → src；无需其他必需 alias。
- Vue 3.5 + Router 4、VueUse core 11.3.0、FloatingVue 5.2.2、OverlayScrollbars 2.15.1、marked 15.0.12、DOMPurify（主线程报告 3.4.16）、Tailwind 3.4、Sass、vite-svg-loader 5.1.0。直接闭包不需要 @floating-ui，FloatingVue 使用其传递依赖。
- 挂载前有 `#teleports`，注册 v-tooltip；主题在 html 首帧设 customdark-mode/brand-hue38；Tailwind content 必须包含 vendor TS 和 SFC。
- WindowControls 只接宿主平台。浏览器必须明确为模拟，Native 不静默回退；隐藏到托盘/窗口状态与真实宿主验证由主线程/用户负责。
- Modal 没有 open model，均调用原 expose show/hide；只有可选 modal-behavior Provider，不需要翻译、网络/业务 Provider。

## 图标名称边界

HomeIcon、LibraryIcon、SettingsIcon、SearchIcon、XIcon、FolderIcon、ArchiveIcon、SquareSplitHorizontalIcon、PlugIcon、UnplugIcon、PinIcon、PlusIcon、ChevronDownIcon、CheckIcon、InfoIcon 均为原存在出口。

HardDriveIcon、DriveIcon、UsbIcon、RowsIcon、ColumnsIcon 在原 assets 导出中不存在，未编造这些别名。适合的原名：**ServerIcon** 实际是 hard-drive SVG；USB 占位用 PlugIcon，安全移除用 UnplugIcon；方向用 SquareSplitHorizontalIcon，消费端对同一原 SVG 旋转 90° 表示上下即可。完整导出名单见精简 generated-icons.ts 和逐文件清单。

## 状态与未验证事项

组件迁移子代理只执行源阅读、复制、接口适配、摘要和许可记录；没有运行检查、服务或 Git 操作。主线程浏览器预览发现并修正 Launcher body 片段漏掉原第66行闭合括号的问题，修改清单已更新。完整前端观察与未验证项统一见 `verification.md`。

未运行本地 build/check/test/lint/tsc/cargo 或 Native dev，不声称 Native 或整个阶段 1 已验收通过。真实触屏、WebView2、Native 窗口与托盘需用户运行后反馈；DropdownSelect 原 falsey-value 行为、NavButton disabled 原 class 语义等接入限制见 integration.md。
