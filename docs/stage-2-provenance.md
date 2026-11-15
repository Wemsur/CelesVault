# 阶段2来源与实现说明

- `crates/vault-core/src/capture/probe/{etw,filter,windows,model}.rs` 提取自本项目已验证的 `tools/wps-probe/src/` MIT诊断叶子；原诊断未改。修改仅包括相对模块导入、独立 `CelesVault-Capture-*`会话、正式健康查询、默认来源工具与不带控制台/样本/原始事件报告的服务接入。诊断MIT说明保留，正式core新增代码按GPL-3.0-only。
- `src-tauri/src/{fs_jobs,path_guard}.rs` 参考 Sigma File Manager 的DirContents分层、复制任务快照、保留两份/跳过/取消设计；未复制整份业务或无界通道，不移植移动/网络/删除。代码是本项目独立实现，GPL-3.0-only。Sigma原参考版本为f0d831bb，原许可证保留于参考项目。
- 通用控件仍使用本项目 vendor 原闭包，未将新业务自制按钮替代原组件。主页/设置/目录视图仅业务组合。
- 新应用品牌是用户2026-11-15提供的图稿；原图稿、派生图与所有图标规格见`branding.md`，不是Celestial品牌。

本文件只说明来源，Native安全行为仍需用户dev验证，不能据此宣称测试通过。机器文件摘要见`history/stage-2-source-manifest.json`；不包括自身或该机器清单的递归摘要。
