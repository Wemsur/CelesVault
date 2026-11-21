# Windows 文件与设备阶段 3 / 6 接口

状态：源码实现，**Native 未验证**。本次未运行构建、编译、cargo、格式化、测试、lint、开发服务、网络、安装、Git 或用户文件。阶段实现修改 `src-tauri/src/fs_jobs.rs`、`path_guard.rs`、`devices.rs` 与本文；按用户最新 app:dev 错误反馈，额外仅在 `src-tauri/src/lib.rs` 增加一行 `mod devices;`。其余宿主命令、前端、核心、依赖声明由父线程/核心所有者接线。

## 依赖与模块声明

宿主声明 `mod devices;`，保留 `mod fs_jobs; mod path_guard;`。

- `serde` / `serde_json`：现有工作区依赖。
- `sha2 = "=0.10.9"`：设备标识和复制审批摘要。
- `windows-sys = "=0.61.2"`，需要以下特性（不新增运行时、安装器、服务或系统设置）：

```text
Win32_Foundation
Win32_Security
Win32_Storage_FileSystem
Win32_System_IO
Win32_System_Threading
Win32_System_WindowsProgramming
Win32_Devices_DeviceAndDriverInstallation
Win32_System_Ioctl
Win32_System_SystemInformation
Win32_System_SystemServices
Win32_System_Com
Win32_UI_Shell
```

`FILE_READ_ONLY_VOLUME` 实际位于 `Win32::System::SystemServices`。没有引入 notify、异步运行时、windows 高层 COM crate 或整套 Sigma。

## 序列化与数据类型

所有公开数据均 camelCase。请求禁止未知字段。`ExportSource` 仅由 Rust 宿主构造，**不实现 Deserialize**，不得新增任意来源路径 IPC。

| 类型 | 字段 |
|---|---|
| DriveInfo | root:String, name:String, identity:String, deviceId:Option<String>, volumeId:String, identityReliable:bool, isUsb:bool, canEject:bool |
| DirectoryView | path:String, identity:String, volumeId:String, writable:bool, entries:Vec<DirectoryEntry> |
| DirectoryEntry | name:String, path:String, identity:String, extension:String, isDir:bool, size:u64, modifiedUnixMs:u64 |
| CopyRequest | targetPath:String, targetIdentity:String, conflict:String（默认 keepBoth） |
| CopyPlan | id:String, items:Vec<ConflictItem> |
| ConflictItem | sourceId:String, sourceName:String, targetName:String, exists:bool, existingSize:Option<u64> |
| ConflictResolution | sourceId:String, policy:String（keepBoth / skip / replace） |
| CopyJob | id:String, state:String, total:u64, processed:u64, copied:u64, skipped:u64, error:Option<String> |
| FileRecovery | id:String, kind:String, originalName:String, heldName:String, heldPath:String, canRestore:bool, error:Option<String> |

`existingSize=None` 序列化为 null；目录存在时亦可显示冲突，但不能替换目录。`extension` 是不带点的小写扩展名；目录为空字符串。目录项 identity 可直接用于该子目录的复制/监听目标；文件 identity 包括卷、文件标识、创建/修改/变更时间、大小、属性、链接数和 GUID 最终路径。

`BrowserEntryLease` 是 Rust-only 类型，有 `pub path:PathBuf`，私有字段保留只读叶句柄、固定父目录链、持久目录保护及设备活动租约，不用于前端序列化。

## FsJobs 完整公开接口

以下均为 `&self` 方法；String 参数不能替换为前端传来的句柄或设备命名空间。

```rust
FsJobs::new() -> Arc<FsJobs>
list_drives() -> Result<Vec<DriveInfo>, String>
browse(path: String, data_root: Option<String>) -> Result<DirectoryView, String>
browse_options(path: String, data_root: Option<String>, show_hidden: bool)
    -> Result<DirectoryView, String>

preview_copy(sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>)
    -> Result<CopyPlan, String>
plan_ids(plan_id: String) -> Result<Vec<String>, String>
start_plan(sources: Vec<ExportSource>, plan_id: String,
    resolutions: Vec<ConflictResolution>, data_root: Option<String>) -> Result<CopyJob, String>
start_copy(sources: Vec<ExportSource>, request: CopyRequest, data_root: Option<String>)
    -> Result<CopyJob, String>
jobs() -> Result<Vec<CopyJob>, String>
cancel(id: String) -> Result<(), String>
busy_volumes() -> Result<Vec<String>, String>
shutdown()

create_folder(path: String, identity: String, name: String, data_root: Option<String>)
    -> Result<(), String>
rename_entry(path: String, identity: String, name: String, data_root: Option<String>)
    -> Result<(), String>
recycle_entry(path: String, identity: String, data_root: Option<String>) -> Result<(), String>

validated_entry(path: String, identity: String, data_root: Option<String>)
    -> Result<PathBuf, String>
lease_browser_entry(path: String, identity: String, data_root: Option<String>)
    -> Result<BrowserEntryLease, String>

file_recoveries(path: String, identity: String, data_root: Option<String>)
    -> Result<Vec<FileRecovery>, String>
restore_file_recovery(path: String, identity: String, recovery_id: String,
    data_root: Option<String>) -> Result<(), String>

watch_directory(path: String, identity: String, data_root: Option<String>,
    callback: Arc<dyn Fn() + Send + Sync>) -> Result<(), String>
unwatch_directory() -> Result<(), String>
watch_error() -> Result<Option<String>, String>
```

除 `new`、`shutdown` 和纯数据类型外，非 Windows 调用返回中文不支持错误，不回退模拟。`browse` 等同 `show_hidden=false`。只有浏览允许尚未选择 data_root；复制、文件操作、监听、系统打开的校验和恢复需要配置后的持久目录，以建立 data 边界。

### 复制结果与失配说明（2026-11-21）

`CopyJob.processed` 是已尝试数量（包含当前失败项），`copied` 只在文件提交后的完整内容复核、同步和目标校验成功后递增。正式文件已提交但后续复核失败时，文件保留且结果明确报错，不因rename返回成功先计作已复制。批次遇失败仍停止，说明失败文件和后续未处理数量，不静默跳过继续。

审批仍严格比较完整FileStamp、路径及SHA-256；失配消息只补充具体文件、字段和旧/新值，不取消ChangeTime、属性或身份检查。未知的用户设备审批失配原因仍待真实反馈。复制流、暂存回读及最终发布回读的SHA-256一致，才通过本次导出内容校验。

### 宿主复制租约交接

1. 预览：从核心获取选中实体的 `ArchiveLease` 和实际显示名，构造 ExportSource；在 `preview_copy` 返回后释放这些租约。计划本身不保留文件/目录句柄。
2. 显示该批 CopyPlan，默认保留两份。全部应用某策略时，宿主为该批 sourceId 构造对应 resolutions；不能仅给旧 start_copy 传 replace 字符串。
3. 批准：调用 `plan_ids`，对其完整 ID 集重新获取核心租约和显示名，传 `start_plan`。FsJobs 再核对来源身份/内容/名称、目标、持久目录及已有目标快照。
4. **所有新 ArchiveLease 必须保留到 start_plan/start_copy 返回**。返回前 FsJobs 已打开整批只读来源并接管；之后宿主才释放租约。

计划最多 8 个，每个最多 1000 个去重来源，300 秒有效；访问时淘汰过期项，新计划淘汰最旧项。无长时间 1000 源文件/冲突文件句柄保留。计划一次消费，失败需重新预览；未知/重复决策 ID、增删来源、来源或已有文件变化均拒绝，不把过期批准改成静默保留两份。省略决策默认 keepBoth；显式 skip 可跳过该项。旧 start_copy 仍支持 keepBoth / 冲突时 skip，绝不接受未批准 replace。

### 显式替换与恢复

替换采用**可恢复的句柄备份协议，不是单次原子覆盖**，原因是 Windows 普通名称替换要求共享 DELETE，而共享后无法严格绑定批准的旧目标。用户明确选择 replace 后：

- 固定旧目标 READ / DELETE 句柄，拒绝其他写入/删除；完整 FileStamp（包含 ChangeTime）和 fresh SHA-256 必须仍与预览一致。目录、只读/系统文件、硬链接和 Windows 系统卷拒绝替换。
- 新 staging 使用 create_new，流式写入并 sync_all；durable 意图记录包含卷代际/目标目录身份、来源 ID、旧快照/摘要、唯一 backup 名称、新 staging 名称/身份/大小/摘要。
- 已批准旧对象按**同一叶句柄**原子改名到唯一 `.celesvault-*.backup`，不覆盖、不预先删除。随后新 staging 按目标目录句柄提交到原名，仍 ReplaceIfExists=false。
- 两次改名之间有短暂原名空窗；任何外部新文件占用原名时立即停止，绝不覆盖或删除它。仅在原名空缺时按旧 backup 句柄恢复。恢复失败明确报告 GUID 备份位置，保留旧内容与意图。
- 新对象身份/大小、磁盘同步和 durable 完成标记确认后，才按本次已知旧句柄删除旧 backup。完成后的记录清理失败与“旧备份仍保留”区分报告，避免虚称已删备份还能恢复。

所有提交/恢复都不使用路径覆盖或预删除 fallback。崩溃/设备异常可能留下 intent、backup、staging、complete 或 resolved 文件；**不全盘扫描、不盲目删除其他 .celesvault 文件、不自动清理用户旧备份**。

`file_recoveries` 只扫描用户当前目录的意图，最多枚举 10000 项、最多返回 256 个恢复记录，每条 JSON 最大 32 KiB。旧 backup 必须匹配文件身份/属性/大小/mtime/摘要、卷代际与目录；原名已有文件则 canRestore=false。`restore_file_recovery` 是单独的用户确认动作，仅不覆盖地恢复原名，并将意图按句柄改为 resolved。若原名已有新导出，可先明确重命名该新文件，再恢复旧文件；本接口不会替用户删除或覆盖新导出。磁盘上的恢复 JSON 是数据，不是任意删除/执行授权。

## 物理设备与弹出接口

```rust
devices::list() -> Result<Vec<DriveInfo>, String>
devices::enrich(drives: Vec<DriveInfo>) -> Result<Vec<DriveInfo>, String>
devices::normalize_volume_identifier(value: &str) -> Result<String, String>
devices::eject(device_id: String, blocked_volumes: Vec<String>) -> Result<(), String>
```

- 枚举本地固定/可移动盘及所有卷；卷使用 `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` 映射到真实物理磁盘，再用 SetupAPI 磁盘接口的 `IOCTL_STORAGE_GET_DEVICE_NUMBER` 做本次运行关联。
- 存储设备描述符提供总线/型号/序列，设备级 StorageDeviceIdProperty 或带 UNIQUEID 且与序列匹配的 PnP 标识提供磁盘级区分。设备 ID 是这些可靠字段的 SHA-256；**不持久化磁盘号、盘符、端口地址或随机制造的“永久 USB ID”**。
- 缺失、占位、格式异常、重复序列号/身份、缺少可靠标识降级为 identityReliable=false / deviceId=None。制造商标识不等于密码学硬件认证；不承诺能识别所有伪造或尚未同场出现的克隆设备。
- 同一物理盘的多个分区共享 deviceId；volumeId 是 GUID + 卷序列 + 文件系统 + 根代际。多物理盘区段卷没有单物理 deviceId。格式化/卷换代不自动套用旧分区固定夹。
- 不可靠 USB 不能通过 None、盘符或其他盘的记录复用固定夹。本地盘允许固定夹：宿主可使用**独立本地卷命名空间**与精确 volumeId 绑定，不把它伪装成可靠 USB hardwareId；重新定位仍由用户确认。
- isUsb 来自存储总线/PnP 关系，USB 的 DRIVE_FIXED 仍被识别。canEject 来自物理 USB PnP 节点的能力/状态，而不是 removable 标签猜测。系统卷、引导/服务分区、程序所在设备以及拓扑不完整的设备禁用弹出。

`blocked_volumes` 必须含**配置后的 data 卷（即使当前无捕获任务）**、核心活动捕获来源卷及 FsJobs 活动卷；任何无法解析的阻塞标识直接拒绝，不过滤。可混用 core GUID 路径、win-dir / win-volume / win-entry 身份，内部统一小写 GUID 根；盘符和磁盘编号不是有效阻塞 ID。

FsJobs 的复制活动涵盖归档源/data 与目标，并展开共享物理磁盘的全部分区和多盘区段传递关系；监听句柄亦作为忙卷返回。文件管理/浏览打开租约还通过内部设备活动注册阻止同模块并发弹出。宿主的配置切换/复制启动/弹出仍应共用业务 mutation 锁。

**弹出顺序**：先停止该目录监听并确认 unwatch 成功，再确认复制/capture 已结束（取消需真正结束），合并全部 blocked_volumes，最后调用 eject。模块再次核对卷区段和物理 PnP 实例，释放查询句柄后只调用 `CM_Request_Device_EjectW`。错误包含 CONFIGRET、PNP veto 类型及有界 veto 名称；失败不记成功。无强制卸载、禁用设备、杀进程、提权、驱动/服务安装。成功表示 Windows 接受正常请求，宿主应刷新设备并清空已移除目标。

## 浏览、文件管理与系统打开

- 每级目录 OPEN_REPARSE_POINT，父链不共享 DELETE；内部访问固定 GUID 路径，另核对显示盘符/当前目录仍指向原对象。盘符/目录/卷代际变化立即错误，不自动换盘。
- 网络、MTP、UNC、设备输入命名空间、相对路径、ADS、保留/歧义组件拒绝；不跟随重解析点。
- 隐藏开关只控制普通 hidden / dot 文件。data、重解析点、系统/临时项、`.celesvault-*`、WPS 锁文件、回收站内部目录仍隐藏。总枚举超过 10000 项明确错误，不悄悄交付不完整列表。
- writable 实际申请 FILE_ADD_FILE / FILE_ADD_SUBDIRECTORY，并检查只读卷，不按设备标签猜测。它不是剩余空间、过滤驱动或每个子项 ACL 的成功保证。
- 新建只 create_dir 单一直接子项，同名报错；重命名固定原对象、按句柄不覆盖改名。drive 根、data 内部/其祖先、系统内部项不可重命名/回收；第一版新建/重命名/替换/回收不修改 Windows 系统卷。data 的外部祖先可浏览并进行普通不覆盖导出，但不能整体改名或回收。
- 回收使用专用短生命周期 STA 的真实 IFileOperation，FOF_ALLOWUNDO + FOFX_RECYCLEONDELETE / EARLYFAILURE，禁用 hooks 和永久 fallback。USB（包括 DRIVE_FIXED 的 USB）、不明总线、非固定盘、系统卷禁用回收。
- 回收保持父链、即时来源快照和拒绝写共享的叶句柄，**允许 DELETE 共享以让 Shell 自己工作**。原名称不改成临时名，系统回收站保留原路径名称。完成后还核实同一文件身份的句柄确实进入该卷 `$Recycle.Bin`；仅 HRESULT 或“原名不在了”不当作成功。Shell 内部仍有同权限恶意并发名称窗口，不宣称绝对 TOCTOU 免疫；不能核实就明确返回错误，且不重复删除。
- 系统打开/Explorer 定位由宿主实现。优先 `lease_browser_entry`，在系统同步请求返回前持有 BrowserEntryLease；path 是校验后的 GUID 路径。仅 `validated_entry` 返回 PathBuf 会释放租约，不提供请求期间句柄保护。该模块本身不执行用户文件或启动任意命令，也不修改归档元数据。

## 真正目录监听与线程结束

监听是 `ReadDirectoryChangesW` **当前目录、非递归的真实 Windows OS watch**，不是全系统扫描/轮询模拟。单 watcher 使用 32 KiB 对齐缓冲、稳定 OVERLAPPED/UnsafeCell、完成/取消事件，startup ready / done 各容量 1；普通通知合并为最多约 200 ms 一次刷新回调，溢出也要求刷新。回调只发送 IPC 提示，不做复制、摘要、数据库重活或无界日志。

每秒只复核该目录/祖先与原卷身份；变化/设备移除保存 watch_error，最后提示宿主刷新并**清当前目标**，不选择另一块盘。宿主应在目录事件中检查 watch_error，并重新浏览获取新快照。

unwatch 设置停止标记/取消事件、请求 CancelSynchronousIo，线程 CancelIoEx 并排空 pending I/O，done 后才 join。公共结束等待最多 2 秒：驱动不响应则关闭回调、保留线程/句柄/缓冲并错误返回，不启动替代 watcher；不能为“及时退出”释放仍被 OS 使用的内存。RAII 排空也覆盖 panic。正常 shutdown 停止 watcher、清计划并取消/join 复制线程；**复制的 cancel 请求不等于 cancelled，实际结束由 worker/join 确认**。坏驱动可能使复制同步 join 等待，这是不能通过强杀或假成功规避的 Windows 边界。

## 待用户 Native 验证

只新增了纯名称、冲突名称、策略/恢复 ID、卷标识规范化与序列号占位判断的源码测试，**没有运行**。需用户完整 dev/手动测试确认：

- 多分区 USB 的物理 deviceId 共享、格式化 volumeId 换代、USB 固定盘识别、不可靠/重复标识不复用收藏。
- 系统/data/capture/copy 任一物理分区阻塞正常 eject，以及实际 PnP veto、拔盘/盘符重占。
- 冲突默认/全批/逐项策略、同批名称去重、审批后目标/来源变化拒绝、单 worker 与计数。
- replace 在 NTFS/FAT/exFAT 的按句柄备份/提交、原名抢占、取消、断电/拔盘、备份恢复与持久意图；目录/只读/硬链接禁止替换。
- 隐藏开关、10000 上限、ACL writable、新建/重命名、系统/data 防护，非 USB 回收真实进入系统回收站，USB 不永久删除。
- 标准 Shell 对 GUID 路径与第三方文件系统的支持；不能解析/回收时必须错误，不能退回未绑定盘符。
- watch 启动错误、突发通知合并、目录/卷变化、溢出、反复切换、2 秒退出及异常驱动资源保留；浏览打开/定位请求期间租约持有。

源码实现、用户 Native 验证、CI/学校验收是三个不同状态，本文件不宣称任何 Native 功能已经测试通过。
