# Windows 构建、Release 草稿与学校部署

状态：工作流及收集/发布脚本已写入，尚未在 GitHub 执行。没有由助手运行构建、测试、Native dev、远程发布或注册系统功能。

## 手动 GitHub Actions

工作流：`.github/workflows/windows-build.yml`，名称“Windows x64 构建与 Release 草稿”。只有 `workflow_dispatch`，不因 push、tag 或 PR 自动触发。

将所需源码和工作流提交、推送至 GitHub 后，在 Actions 选择该工作流 → Run workflow，选择要构建的分支。手动入口通常需要工作流已进入默认分支；不要把本地新建文件当成远程已有工作流。工作流与锁文件现已本地提交；远端可用性及执行结果尚无验收记录。助手不代为推送或触发。

### 版本和 Tag

根 `package.json` 的 `version` 是版本读取入口，Tag 固定为 `v` 加版本号，当前为 `v0.2.0`，不提供手填 Tag 的覆盖入口。构建前同时核对：

- 根 `Cargo.toml` 的 `[workspace.package].version`；
- `src-tauri/tauri.conf.json` 的 `version`。

三处必须一致，支持如 `0.2.1-beta.1` 的预发布版本；禁止空值和任意路径/命令字符。版本更新后由用户正常依赖解析更新 Cargo.lock，不在 CI 自动修改版本或依赖锁。`Cargo.lock` 和 `pnpm-lock.yaml` 必须纳入提交，否则预检失败；构建用冻结/锁定选项，不在失败后自动放宽。

Tag 已存在时只允许它指向本次构建的精确提交，绝不移动已有 Tag。相同版本已有 Release（包括草稿）时中止，不覆盖附件、不修改已公开版本。需要重试草稿上传失败时，先人工检查并处理不完整草稿，或提升版本后重新构建。

### 构建及产物

Windows 2022 x64 托管 runner，Node 24、仓库声明的 pnpm 10.15.0、Rust stable MSVC，缓存 pnpm/Rust；仅构建 `x86_64-pc-windows-msvc`，不运行应用、ETW 或系统集成测试。前端通过 Tauri beforeBuildCommand 构建嵌入。

交付文件：

- `CelesVault-<版本>-windows-x64.exe`：便携程序；
- `CelesVault-<版本>-windows-x64-setup.exe`：NSIS 安装器；
- `CelesVault-<版本>-source.zip`：同一精确提交的 Git 跟踪源码，保留 vendor、构建脚本、锁与许可；不打包 data、target、握手令牌或本地日志；
- `LICENSE`、`THIRD_PARTY_NOTICES.md`、`RELEASE-NOTES.md`；
- `build-info.json`：版本、Tag、完整提交、目标和运行号；
- `SHA256SUMS.txt`：以上各文件 SHA-256。

源码包不是离线依赖镜像，依赖依照锁文件联网取得，发行时仍须履行第三方许可和完整对应源码义务。未配置代码签名，不声称可信发布者或绕过 SmartScreen。

Actions 保存一份30天的产物；构建 job 成功后，独立发布 job 重新校验摘要及版本/提交，使用内置 `GITHUB_TOKEN` 的 `contents: write` 创建 GitHub Release **草稿**并上传上述附件。构建 job 仅 `contents: read`，不传发布令牌给构建命令。无需填个人 PAT、GPG 或其他发布站点令牌。仓库/组织若限制写权限，发布 job 会失败而不会改用其他凭据。

草稿不会自动改成正式发布或 Latest；只有人工查看、验证并决定发布。上传中断时保留未公开草稿并明确失败，Actions构建产物仍可下载。本次只是配置该流程，没有创建远程 Tag 或 Release。

## 部署边界

- 第一版整个进程管理员运行，manifest 要求 `requireAdministrator` 并核对 Token。完整 dev 由用户在主动打开的管理员终端运行，不降低 UAC 或执行策略。
- WebView2 是 Windows UI 的运行环境前提；便携单 EXE 不意味着不需要 WebView2。NSIS配置明确使用 `webviewInstallMode: skip`，不自动下载或安装WebView2；缺少运行环境需先由用户/学校确认准备。本次没有在开发机安装运行环境。
- EXE 同级 data 为首次目录选择建议。资料、SQLite、课表、排除、收藏和界面设置存于选定持久根；定位文件在程序同级。学校选择不受还原影响的 D 盘目录。
- 普通关闭隐藏到托盘；主动退出停止后台。蓝/绿托盘活动提示不是保存完整性替代，只有真实提交成功才能算已归档。
- 不安装服务、自启动或 WPS 预览，不改文件关联，不在卸载时主动删除外置资料 data。NSIS 实际安装/卸载仍需验证。
- 自启动仅经正常维护更新冻结基线或使用已有获准持久启动链；D盘存在EXE不代表会自动启动，不绕过冰点。

## 待验收

- [ ] 首次手动运行构建，确认依赖锁、MSVC、前端和 NSIS 均成功；助手不代为触发。
- [ ] 构建产物版本一致，EXE/NSIS真实存在，校验和正确，源码包包含本次提交的构建文件、vendor及许可。
- [ ] Release 为未公开草稿，Tag来自仓库版本且目标提交正确；重复版本或错指Tag不会覆盖。
- [ ] 便携EXE与安装版管理员启动、WebView2前提、中文界面/字体、托盘及数据根恢复。
- [ ] 安装、更新、卸载不删除外置data；旧库迁移前正常退出并备份，不能用旧程序打开新schema。
- [ ] Windows10希沃/WPS预览、真实触屏、多分区U盘、盘符变化、正常弹出与学校冻结后资料恢复。

源码完成、CI构建完成、本机Native验收与学校验收分别记录，不能互相替代。
