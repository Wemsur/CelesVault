# 架构与阶段边界

## 阶段 1

Vue 3 + TypeScript + Vite 7 + Tailwind 3.4；Vue Router 使用 hash 路由，Pinia 只持有当前壳层会话状态。通用组件直接来自 Celestial 工作树，文件来源与适配见 `component-provenance.md`。全文案为普通中文字符串，没有翻译 Provider。

应用壳沿用原顶部 3rem、左侧 4rem、48px 圆形导航、内容左上 `--radius-xl`。主页无额外“今日文件”标题/宣传副标题，无外边距，内容直接左右（或上下）分成两半；课程和导出目标内部再组织，不增加顶部引导区。主题首帧在 html 设置 `dark-mode customdark-mode` 和 `--brand-hue:38`，Teleport 到 body 仍继承同主题。Inter 3.19 的 400–800 五字重为本地 WOFF2，中文沿用原字体栈的系统回退；没有新增字体 CDN 请求。

## 平台接口

- `src/platform/types.ts`：当前窗口/运行状态能力契约。
- `native.ts`：调用真实 Tauri 窗口命令与事件，不导入 PreviewAdapter。
- `preview.ts`：只在开发浏览器中使用，窗口动作明确显示模拟结果；不读取驱动器、教学资料或正式 data。
- `index.ts`：显式检测 Tauri。正式静态页面无 Tauri 时拒绝启动，而不是静默演示成功。
- 页面从 `usePlatform()` 获取能力，不直接调用 Tauri；原窗口控件通过平台接口适配。

截至2026-11-21，Native已接监测/归档、配置与课表、全历史库查询/受限正则、图片缩略图、回收恢复、物理设备固定夹、批次复制审批、目录监听与安全弹出。方向、缩放与分屏/窗口状态存于持久根。浏览器系统能力仍为false且拒绝真实文件操作；源码接入不代表Native验收。

## Native 宿主

Tauri2承担窗口/托盘和真实命令边界；`crates/vault-core`无Tauri依赖，阶段2接SQLite、ETW、归档意图及重试。`tools/wps-probe`保持独立Cargo工程、不受workspace干扰。目录与导出任务在宿主独立fs模块，源按资料ID/摘要租约，目标按句柄身份固定。

Windows manifest 请求管理员权限；运行时再核对当前 Token，不安装服务、启动任务、系统关联或预览组件。阶段2托盘接打开、监测暂停/恢复与退出；未配置时禁用暂停，异常/复制失败仅变色。窗口关闭隐藏，程序仍驻留；托盘退出才退出进程。原生关闭与窗口状态由 Rust 控制，显示/还原是用户主动动作。

权限仅供本地主窗口；没有通用文件系统插件、远程 origin 权限、联网 API、Shell 执行接口或远程页面。CSP 不复制原 Launcher 的 Modrinth/广告/支付/遥测白名单。

## 阶段2与后续

阶段2早期历史见`stage-2.md`；阶段3–6当前接口与安全边界见`core-stage-3-6.md`及`files-stage-3-6.md`。阶段6仍需真实触屏/复制/故障恢复与性能收口，阶段7工作流已配置但CI/学校未验收。空态不填假资料或假磁盘。外观扩展见`appearance.md`。
