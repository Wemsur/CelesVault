# 阶段 3–6 核心数据 API 与持久化契约

状态：2026-11-15 核心实现已落码；这里只记录 `crates/vault-core/**` 的实际接口与行为，不代表 Native、测试或学校验收通过。前端、Tauri 命令桥、文件浏览/设备/复制及其他文档由各自所有者负责。

## 1. 公开类型

权威定义为 `crates/vault-core/src/api.rs`，由 `lib.rs` 在 Windows/非 Windows 上统一导出。Rust 字段为 snake_case，序列化为以下 camelCase。可选字符串为 `Option<String>`，序列化 `None` 为 `null`。

| 类型 | 完整字段与类型 |
|---|---|
| EngineStatus | dataRoot?: String；monitorState、monitorDetail: String；pendingCount、failedCount: u64；hasErrors: bool；health: String[] |
| SourceRule | path: String；recursive: bool |
| CaptureSettings | mode: String；processNames、processPaths、wpsInstallPaths: String[]；sources: SourceRule[]；includeDefaultSources、includeUsb: bool；extensions、excludedPaths、excludedSubjectIds: String[] |
| Subject | id、name: String；color?: #RRGGBB字符串（旧配置默认为null，前端默认配色） |
| TimeSlot | id、label、start、end: String；isClass: bool |
| WeekLesson | weekday: u8；slotId、subjectId: String；enabled: bool |
| Timetable | subjects: Subject[]；slots: TimeSlot[]；lessons: WeekLesson[] |
| AppSettings | homeDirection: String；horizontalRatio、verticalRatio、horizontalTargetRatio、verticalTargetRatio、libraryRatio: f64；scalePercent: u32；showHidden: bool；browserView: String；windowWidth、windowHeight: u32；windowMaximized、closeNoticed: bool；notificationSeconds: u32（旧配置默认6，范围0–120，0手动关闭） |
| FixedFolder | id、deviceId、volumeId、relativePath、alias: String；position: u32 |
| ConfigBundle | version: u32；app: AppSettings；capture: CaptureSettings；timetable: Timetable；favorites: FixedFolder[] |
| ArchiveItem | id、displayName、extension、state: String；size、detectedUnixMs: u64；taskId?、error?、courseId?、subjectName?: String |
| CourseView | id、date、subjectName、label: String；slotId?、subjectId?: String；startUnixMs、endUnixMs: u64；enabled: bool；items: ArchiveItem[] |
| LibraryQuery | name: String；regex: bool；subjects、extensions: String[]；dateFrom?、dateTo?: String；groupBy、sortBy: String；descending: bool；offset: u64；limit: u32 |
| LibraryRow | groupKey、groupLabel: String；item: ArchiveItem |
| LibraryPage | rows: LibraryRow[]；total: u64 |
| UsageDetail | date、courseId、subjectName: String；detectedUnixMs: u64 |
| ArchiveDetail | item: ArchiveItem；archivePath: String；sourcePaths: String[]；uses: UsageDetail[]；deleted: bool |

原 `Capabilities`、`RuntimeInfo` 与阶段 2 的 `ArchiveItem` 原字段保留。新增 `courseId`/`subjectName` 不替代原 taskId/error/state。

## 2. VaultEngine 最终接口

所有操作方法均同步；宿主应对数据库/文件/摘要/缩略图调用使用 `spawn_blocking`，而非阻塞 UI。以下为实际 Rust 签名，不包含私有 helper。

```rust
pub fn new() -> Arc<Self>;
pub fn status(&self) -> Result<EngineStatus, String>;
pub fn configure_root(&self, path: String) -> Result<EngineStatus, String>;
pub fn restart_monitoring(&self) -> Result<EngineStatus, String>;

pub fn config(&self) -> Result<ConfigBundle, String>;
pub fn save_config(&self, bundle: ConfigBundle) -> Result<ConfigBundle, String>;
pub fn import_config(&self, text: String) -> Result<ConfigBundle, String>;
pub fn export_config(&self) -> Result<String, String>;
pub fn import_timetable(&self, text: String, format: String) -> Result<Timetable, String>;
pub fn export_timetable(&self, format: String) -> Result<String, String>;

pub fn courses(&self, date: String) -> Result<Vec<CourseView>, String>;
pub fn list_today(&self) -> Result<Vec<ArchiveItem>, String>;
pub fn change_course(&self, course_id: String, subject_id: String) -> Result<(), String>;

pub fn library(&self, query: LibraryQuery) -> Result<LibraryPage, String>;
pub fn detail(&self, id: String) -> Result<ArchiveDetail, String>;
pub fn edit_archive(&self, id: String, name: String, course_id: Option<String>) -> Result<(), String>;
pub fn move_usage(&self, id: String, from_course_id: String, to_course_id: String) -> Result<(), String>;

pub fn favorite_save(&self, folder: FixedFolder) -> Result<(), String>;
pub fn favorite_remove(&self, id: String) -> Result<(), String>;

pub fn trash(&self, id: String) -> Result<(), String>;
pub fn trash_list(&self) -> Result<Vec<ArchiveDetail>, String>;
pub fn restore(&self, id: String) -> Result<(), String>;
pub fn purge(&self, id: String) -> Result<(), String>;
pub fn busy_volumes(&self) -> Result<Vec<String>, String>;
pub fn thumbnail(&self, id: String) -> Result<Option<String>, String>;

pub fn pause(&self, paused: bool) -> Result<EngineStatus, String>;
pub fn retry(&self, task_id: String) -> Result<(), String>;
pub fn dismiss_failed(&self, task_id: String) -> Result<(), String>;
pub fn archived_path(&self, id: String) -> Result<PathBuf, String>;
pub fn lease_archive(&self, id: String) -> Result<ArchiveLease, String>;
pub fn get_capture_settings(&self) -> Result<CaptureSettings, String>;
pub fn set_capture_settings(&self, settings: CaptureSettings) -> Result<(), String>;
pub fn source_defaults(&self) -> Result<Vec<SourceRule>, String>;
pub fn shutdown(&self);
```

`dismiss_failed` 仅接受当前失败任务，无活动归档/回收意图时将失败任务设为dismissed并从今日/库列表撤下；同一失效实体的失败记录一起撤下。保留任务、别名、实体与使用历史，不执行任何来源/归档/暂存文件IO，不视为移入回收站。已撤下任务不会因普通重启或重复事件重新启用，也不能再从此卡片手动重试。该状态使用现有tasks.state，无表结构迁移。

`import_config`、`export_timetable`、`move_usage` 为集成契约之外已同步给消费端的补充方法。导入方法只返回已验证草稿，不选择根、不落库；必须经过一次显式 `save_config`。

`ArchiveLease` 公开 `path: PathBuf`（已固定的 GUID 路径）及 `display_path() -> Result<PathBuf,String>`（重新验证后的当前 DOS 路径）。复制方必须保留**整批租约直到所有源句柄已打开/固定**；系统打开方也须保留租约直到系统动作完成。`archived_path` 保留阶段 2 兼容签名，但不能替代新消费端的 caller-held lease。租约允许全部可用历史实体，没有“仅今天”限制。

非 Windows：所有相同类型/方法均导出，操作返回明确“不支持真实 Windows 能力”错误；status 明确 error/hasErrors，不提供模拟成功。独立公开的纯验证/导入函数可用于跨平台单元测试源。

## 3. 配置验证与一次保存

配置格式 `CONFIG_VERSION=1`，与 SQLite schema 版本分开。

- `homeDirection` 为 horizontal/vertical；`browserView` 为 grid/list。
- 五项比例均要求有限数值、范围 `[0.15,0.85]`；默认 `.5`。缩放整数 75–200，默认 100。
- 窗口宽 1100–7680、高 700–4320；默认 1280×800。默认 showHidden/windowMaximized/closeNoticed 为 false。
- 科目color可省略或为null；设置时须为#RRGGBB，规范成小写。CSV接受原10列或追加color的11列，导出为11列，仅subject行使用color。该字段只负责显示，不重算课次历史。
- 默认 Timetable/favorites **为空**，不建立示例课程或资料。
- 科目/时间段/固定夹 id 为 1–80 个 ASCII 字母、数字、短横线或下划线。科目名、时间段标签、固定夹别名最多 128 字。
- favorites 最多 512 项，position 为 0–4095；相同设备/卷/相对路径不得重复。相对根用空字符串，不接受绝对路径、`..`、ADS、保留组件或歧义路径。
- 新固定夹 volumeId 严格采用设备模块完整代际串 `win-volume-v1|\\?\Volume{GUID}|{serial8hex}|{filesystem}|{rootIndex16hex}:{rootCreated16hex}`，五段完整验证后**原样保留**，不截 GUID 或丢弃格式化代际：serial 为 8 位 hex，rootIndex/rootCreated 各 16 位 hex，filesystem 为 1–32 个 ASCII 字母/数字/短横线/下划线，GUID 段无尾斜线。不同 serial/FS/rootIndex/rootCreated 的代际不会被合并成同一卷记录。
- 旧裸 `\\?\Volume{GUID}` 仅可作为旧配置记录导入/保存，不补猜 serial、FS 或 root 信息，不自动升级或匹配新的 generation 身份。deviceId 必须非空且不含控制字符；宿主须按当前 DriveInfo 完整 deviceId/volumeId 核对后保存/定位，不能仅按 GUID 自动认领另一代设备。
- root/busy_volumes 的 GUID 规范化仍使用独立 `config::volume_id`，输出 canonical `\\?\Volume{小写UUID}` 前缀且无尾斜线；不能把该裸 GUID 结果用于固定夹代际匹配。ConfigBundle 仍为 version 1，缺字段导入明确错误，不为旧记录补猜新身份。
- capture 保留 strict/loose、目录递归、USB/默认源、扩展名、进程名称/路径、安装目录、排除规则。每类最多 128 项；禁止 Explorer/dllhost/System 等通用主程序来源。路径只作严格本地盘符词法验证，设置中的离线源不要求保存时已挂载；捕获打开时再验证真实卷/文件身份。
- 配置/课表导入文本及完整序列化配置各不超过 1 MiB。嵌套 JSON 拒绝未知/缺失字段、不支持的版本及非法值，不静默补默认或夹取上下界。

`save_config` 先验证整个 bundle，再在单个 SQLite 事务保存 bundle 与阶段 2 capture 镜像；提交成功后才更新内存设置。旧 `set_capture_settings` 和 favorite CRUD 也经同一个完整保存逻辑。没有 dataRoot 字段，不能通过保存配置选择/搬迁持久根；未 configure 时明确失败。

UI 保存/取消/未保存提示与预览回退由宿主管理。自动保存 app 视图需在宿主 mutation 锁内读取最新 config，仅 patch app 后一次保存，不能用旧 bundle 覆盖课表/固定夹。

配置导出仅包含 app/capture/timetable/favorites，不包含实体、使用记录、SQLite 或归档字节，**不是完整灾备**。

## 4. 时间表、快照与首次归属

- 日期严格 YYYY-MM-DD，年份 1970–9999；本地时刻严格 HH:mm。Unix 毫秒须可表示；本地夏令时缺口/歧义明确拒绝，不猜时区边界。
- 最多 256 科目、128 共用时间段、896 星期课程。每日全部时间段（包括非课段）按开始时间排列、开始小于结束、不重叠且不跨午夜。
- weekday 使用 ISO 1–7。星期/slot 唯一；只可给 isClass 的段配置课程。启用课必须引用存在科目，禁用空科目允许保存。
- 新日期首次请求/捕获按当前配置生成并保存当天启用课次及未归类日界桶。已有课程快照不随普通周课表修改重算；相同科目多节仍有不同 id。
- v1 既有日期的使用保持原日期并迁为未归类快照，不补猜过去的课表。尚无课程快照的当日/未来日期可补建配置中的启用空课次，旧未归类记录**不会自动迁过去**。
- `courses(date)` 保留启用的空/未开始课程、开始/结束毫秒与稳定 course id；非课程/禁用课不显示，其他资料在当日未归类桶。未来态是 UI 呈现，不限制已有可用准备资料的租约/导出。
- 接受事件用原 ETW 首次有效时间，不用 observed 时间或复制成功时间。课中归本课；课前/课间归当日下一节启用课；最后一课结束后/无课日无下一节时未归类，不跨日期。结束边界为排他。
- tasks 的源身份唯一键是 `(course_id,identity)`；usages 是 `(course_id,entity_id)`。同内容同日不同课次各自记录，同课次高频合并；乱序更早检测时间取 min，不延长重试、不自动重开失败任务。
- 提交复制成功时重新读当前 task 首次时间与课程快照名，避免迟到 worker clone 覆盖当天调课。重试仍保留最初日期/身份/课次，有限重试窗口 40 分钟。

`change_course(id,subjectId)` 只作用今天：更新课程科目、该课既有使用与捕获关联的科目名，周课表与其他日期不变。空 subjectId 表示取消今天这课：快照 enabled=false，关联事务转当日未归类，物理文件不移动。活动归档/待重试任务或同源 identity 唯一键冲突时拒绝**整笔**操作，不通过删任务历史强行合并。非空 subjectId 可重新启用该课，但不会自动迁回先前转至未归类的记录。

## 5. 实体位置、名称与纠正归属

新实体按首次捕获日期/课次安全标签保存：

```text
archive/日期/课次序号-安全科目名/安全原名主体--实体UUID.真实扩展名
archive/日期/未归类/安全原名主体--实体UUID.真实扩展名
```

相同摘要/大小复用首个实体，后续课次只增使用；因此并非每个日期/课次都再有同内容文件。旧 v1 `未分类` 等路径保持原样，不移动旧实体或原文件，不使用原文件硬链接。

显示名为实体级元数据；原名/来源别名继续留在 tasks/usages/aliases。`edit_archive(id,name,None)` 只改显示名，name 必须是安全单组件并保留真实格式扩展名；不改摘要、实体文件名、位置或原文件。Some(courseId) 只纠正该实体最近一条使用，目标须同日期/启用；其他日期历史不变。更精确的 `move_usage(id,from,to)` 指定源记录。目标已有同实体、不同源身份时合并最早使用；同源任务身份冲突则整笔拒绝，不丢任务/别名历史。

库导出文件名应从 `detail(id).item.displayName` 冻结取得，不从档案 UUID 文件名猜测。批次 entity 去重与目标冲突批准由文件模块负责。

## 6. 真正元数据库查询与详情

- 仅 SQLite 元数据，不扫描原文件/全文。候选包含 available/failed 实体历史，排除回收中、已回收和 purged；不是 today 的代理。
- name 匹配实体名、课程原名、全历史来源别名。普通关键词与 regex 显式分开；Rust regex **1.12.2**，大小 512 字节、编译程序 256 KiB、DFA 512 KiB、嵌套 32；大小/语法/不支持的 JS lookaround/backreference 明确报错。无 JS 或灾难回溯引擎。
- subjects 可用快照科目 ID/科目名；未归类 ID 使用 `unclassified`。日期范围包含两端，筛选应用于同一使用记录；extensions 小写、空字符串代表无扩展名，各列表最多 128 项。
- groupBy 为 none/date/month/subject/course/type；sortBy 为 archived/used/name/size，descending 决定方向。
- limit 为 1–200，offset ≤1000000；默认 100/0。SQL window 按 `(entityId,groupKey)` 去重后 LIMIT/OFFSET，排序追加 entityId/groupKey 稳定次序。total 是去重后的**分组行数**；同实体可以跨组显示，但复制批次不能重复导出。
- archived 用实体首次成功收纳时间；used 用该组最新匹配课次的首次检测时间；ArchiveItem 的课程/科目取该组代表使用。
- available 是已提交保存状态；库查询不为每行做磁盘摘要。今天/课程查询保留阶段 2 的完整指纹缓存校验；真正打开/租约/导出仍 fresh SHA。
- detail 返回全历史 uses/sourcePaths、当前显示名/大小/状态，以及原数据卷重新验证的 DOS archivePath；无 DOS 挂载时可返回 GUID 信息路径，但系统打开明确失败。回收中断则按已知 from/to 的当前位置显示，不把不存在的恢复目标假装成已移动成功。sourcePaths 只用于历史，不提供打开原件。

## 7. SQLite schema 3 与旧版本迁移

`application_id=0x43564C54`、`user_version=3`，rollback journal PERSIST、synchronous FULL。DB、journal、根锁与祖先目录句柄保持阶段 2 的 reparse/hardlink/换卷保护。

| 表 | 关键内容/键 |
|---|---|
| config | key/value；bundle（配置格式 1）与 capture 镜像同事务 |
| entities | 原 id/digest/size/rel/state/error，增加 display_name/extension/archived/owned_identity/trash_rel/deleted_at；rel 唯一；content 唯一键保留至 purged |
| day_snapshots | date 主键、created、has_schedule |
| courses | 日期/slot/科目/名称/标签/开始结束/enabled/ordinal；id 主键、日期/slot 唯一；科目快照不依赖现周课表存在 |
| tasks | 保留源/身份/首次时间/截止/重试/staging/final/entity/digest 等，新增 course_id/subject_name；唯一 `(course_id,identity)` |
| usages | 日期、科目快照、首次检测、task 引用、原名；主键 `(course_id,entity_id)` |
| aliases | 日期/课次/源身份/原路径/原名；主键 `(course_id,identity,source,display_name)` |
| file_intents | 每实体一个显式 trash/restore/purge 意图，from/to、已验证身份、摘要、大小、时间、错误 |
| thumbnails | 实体、source 摘要、PNG 摘要、自有文件身份、相对路径、大小、创建/使用时间；LRU 索引 |

v1 迁移在一个 IMMEDIATE 事务中重建任务/使用/别名键：保留所有原 ID、摘要/大小、source/identity、detected/date、deadline/attempts、stage_rel/final_rel/entity_id 与记录数量；外键一致后才提交 schema 版本。失败 rollback，不重置历史或文件。不是本机已执行/验收的迁移结果。

v1 无收纳完成时刻，archived 只能以已有最早检测时间回填，不能声称知道原复制完成时间。v1 无 OWN file identity 列，仅在 fresh SHA/只读/独立文件验证成功后建立身份基线；无法追溯识别**迁移前**已发生且字节完全相同的替换。新实体从首次落盘就记录真实身份。

2026-11-20 新增科目排除：schema v3 为 tasks 增加 `attribution_known`，v1/v2 迁移时默认未知；有排除规则时，不能可靠归属的旧未归类任务保守阻止，不猜今天的课程。迁移源码尚未执行验证。

旧程序不支持 schema 3；配置 JSON 导出不能用来降级/恢复数据库。跨卷复制等导致文件 ID/卷身份变化时不会自动重新认领或迁移；同卷保留文件身份的目录改名/移动可使用相对路径。不能把未知替代文件当成自己的副本处理。

## 8. 回收、恢复与活动保护

- trash 是按 OWN 验证句柄不覆盖移动至 `recovery/实体UUID/动作UUID.cvtrash`；原路径/使用/源别名保留。
- 状态迁移：available/failed → deleting → deleted；deleted → restoring → available；deleted → purging → purged。异常实际文件须仍通过身份/内容验证才能处理，不能借删除按钮强行删除外部替代物。
- purge 只针对明确回收的资料；删除已验证自有 payload 与已知自有缩略图，保留元数据 tombstone/使用历史。没有自动清档，原文件不删除。已知回收文件缺失时，显式 purge 可只清理其状态/已知缓存；没有拥有身份则不猜。
- 每个动作先事务记录意图/不可用状态，IO 后再事务完成；进程重启只重放已有意图。源/目标同时存在、都缺失（移动）、身份/摘要变化、目标被占等明确失败并保留意图/error，status/trash_list 可见；调用原动作可重试。
- purged/deleted/过渡实体无法租约、打开或生成缓存图。staged/copying/pending 的实体与活动读/写租约拒绝删除/恢复/清理。恢复可用前在状态锁释放前关闭私有 writer。
- OWN 操作从不通过路径解除只读：只有 identity+SHA 校验成功、独占、单硬链接的原句柄可改变自身属性/删除。外部换成同名甚至同内容文件仍拒绝；不会覆盖冲突恢复目标。
- 生命周期串行化最外层入口，内部 helper 不递归获取同一生命周期锁。长期摘要/图片解码在状态锁外；后台只持 Shared，不形成 Engine Arc 自循环；start 拒绝覆盖活动 Service，stop 同步 cancel+join。中断 copying 保留/重放原意图而非改日期。
- busy_volumes 包含永久 data 卷、pending/copying/staged 源身份及仍活动的候选/复制读源卷，输出 canonical GUID 前缀且无尾斜线；宿主须合并 FsJobs 的活动源/目标卷，再交设备模块按全部分区阻止物理弹出。核心不调用强制卸载。

trash/purge 的用户二次确认由 JS/宿主在命令前执行；这里没有绕过确认的 UI，也没有替用户安装服务、自启动或关联。

## 9. 图片缩略图

`thumbnail(id)` 是真实同步计算接口，由宿主异步 spawn_blocking 调用；返回 PNG data URI。不支持格式返回 None，坏图/过限/缓存异常返回实际错误。

- image 0.25.10 只启用 png/jpeg/webp/gif/bmp/tiff；Office/PDF、HEIC/ICO 等没有假首页缩略图。GIF 只首帧。
- 在解码前检查编码文件 ≤64 MiB、尺寸乘积 ≤32000000、单边 ≤32768，设置 decoder/output 分配预算 128 MiB；不声称这已经是测出的进程峰值内存。
- 最大 256×256 PNG、单 PNG ≤1 MiB。读取已 fresh 验证的 archive lease，不读取来源 USB 原件。
- data/cache/thumbnails：只新建 UUID 自有缓存、回读摘要、只读保护、记录 identity/PNG/source digest。已知缓存 LRU 清理也须 OWN 验证句柄；全目录物理预算 32 MiB/256 文件，未知文件计入预算但不删除，扫描最多 1024 条。
- 缓存索引前崩溃可能留下未知孤立缓存；不猜归属清理。预算被未知文件占满时返回实际生成 data URI，同时 status 记录“未写缓存”错误，不扩大缓存或写另一目录。缓存缺失/被替换/摘要错误明确返回，归档本体仍走自己的打开/导出验证。

## 2026-11-21 外观配置扩展

AppSettings新增默认appearance，字段为theme/hue/background/shader/parameters/image/imageFit/imageOpacity/imageBlur；每个背景独立quality/speed/intensity/fps/scale/detail/grain/density/shockwave。旧配置缺appearance时默认customdark/hue38/off；配置格式与schema不变，Rust校验枚举/有限范围、有界JPEG副本，仍受整份1MiB限制。见appearance.md。旧配置和边界测试仅新增源码未执行，Native重启持久化待验收。

## 10. 依赖、只读核对与未验收

核心 manifest 精确复用本地已有版本：regex 1.12.2、csv 1.3.1、image 0.25.10、base64 0.22.1；rusqlite 0.37.0 增 functions；chrono 0.4.42 移通用依赖。原 sha2/uuid/windows-sys 版本及 Win32_System_Time 等 features 保留。根 Cargo.lock/宿主/workflow 不由本核心任务修改。

实际核对仅为源码阅读与定点检索：公开字段、Windows/非 Windows 方法表、事务列映射、限定依赖、句柄生存范围与错误路径。未运行任何构建、cargo check、测试、类型检查、lint、format、dev、服务、安装、联网或 Git 命令。

已有纯 Rust 单元测试**源**位于 config.rs/timetable.rs/library.rs：日期/闰日、每日时间合法性/重叠/引用、同科目多节/禁用、课前课间放学/跨日、CSV/JSON 回导、空默认/未知字段、窗口比例缩放/固定夹安全边界、完整卷代际 JSON 回导/非法分段/格式化代际保留/缺字段拒绝、受限 Rust 正则与分页范围。没有执行，不能称通过。

主线程转达用户 dev 输出“vault-core generated 7 warnings”，未给出明细；只清理源码已明确确认未调用的内部方法，不猜是哪七条，不声称全部消除。文件系统编译错误由文件模块所有者处理，不作为本核心 Native 验收。

仍须用户 Native 验证：v1 实际升级且历史/路径不丢、真实课表/课间/跨午夜/当天取消、延迟复制调课与同内容多课次、配置重启/导入取消、真实全历史分页/正则、拔源盘打开/导出、活动租约拒绝删除、回收/恢复/永久清理中断重启、外部替换/硬链接/junction、GUID/盘符/设备换代、六类图片边界与缓存、10k 元数据/8GB 触屏设备资源目标。这里只交付核心代码与接口，不将 Native、CI、学校或性能目标记为已通过。
