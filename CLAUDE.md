# CelesVault 工程约束

- 工作范围以 `docs/requirements.md` 和 `docs/implementation-plan.md` 为准。2026-11-15 用户授权替换正式图标并继续下一步，当前范围扩展至阶段 2 最小真实收纳/复制闭环；不自动推进阶段 3。
- **不自行运行构建、编译、cargo check、测试、类型检查、lint 或完整 Rust/Tauri dev。** 用户自己运行并反馈。允许仅前端浏览器预览；预览不等于 Native 验证。
- 入口：`pnpm run web:dev` 为仅前端，`pnpm run app:dev` 为用户运行的完整开发。
- 全中文；不引入 i18n、语言包、翻译 Provider、语言设置或单语言翻译壳。
- Celestial Launcher 原组件、customdark 色相 38 与用户截图为最高视觉基准。复制必要源文件，不用自制近似组件替代；来源与适配记录到 `docs/component-provenance.md`。
- 已验证阶段可本地提交。**所有新提交禁止 Co-Authored-By**；原诊断历史保留原作者、时间、消息。推送、PR、发布需用户明确授权。
- 此目录已独立 Git。上层 `github-project` 仍有其他项目暂存内容及旧 CelesVault 跟踪；不对父仓库执行 add/reset/clean/stash/commit 或历史过滤。
- 少量子代理需职责清楚、写入范围不冲突；不大规模编排。
- 联网失败报告具体地址/错误，等用户处理代理/TUN，不反复盲试。
- Native 环境不能静默回退预览；未实现的监测/归档/复制/弹出不得模拟成功。
- 第一版整个应用管理员运行、关闭到托盘；不擅自安装服务、自启动、WebView2、WPS 预览或修改系统关联。
- 使用适用 GPLv3 条件，保留第三方许可与对应源码义务；独立诊断工具继续保留自己的 MIT 声明。
