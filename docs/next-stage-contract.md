# 阶段3–6集成契约（实施中的内部边界）

camelCase序列化；日期YYYY-MM-DD，本地时间HH:mm，星期ISO1–7。只有实际Native功能接入后才宣称能力存在。接口可在必要时具体澄清，主线程同步消费端；不得模拟Native成功。

## 数据核心（vault-core所有者）

保留阶段2接口。新增：

- Subject{id,name}；TimeSlot{id,label,start,end,isClass}；WeekLesson{weekday,slotId,subjectId,enabled}；Timetable{subjects,slots,lessons}。
- AppSettings{homeDirection,horizontalRatio,verticalRatio,horizontalTargetRatio,verticalTargetRatio,libraryRatio,scalePercent,showHidden,browserView,windowWidth,windowHeight,windowMaximized,closeNoticed}，默认比例.5、scale100、browserView grid。
- FixedFolder{id,deviceId,volumeId,relativePath,alias,position}。ConfigBundle{version,app,capture,timetable,favorites}。
- CourseView{id,date,slotId?,subjectId?,subjectName,label,startUnixMs,endUnixMs,enabled,items:ArchiveItem[]}；ArchiveItem新增courseId?/subjectName?，原字段保留。
- LibraryQuery{name,regex,subjects:string[],dateFrom?,dateTo?,extensions:string[],groupBy,sortBy,descending,offset,limit}；LibraryRow{groupKey,groupLabel,item:ArchiveItem}；LibraryPage{rows,total}。sortBy archived/used/name/size；groupBy none/date/month/subject/course/type。
- UsageDetail{date,courseId,subjectName,detectedUnixMs}；ArchiveDetail{item,archivePath,sourcePaths:string[],uses:UsageDetail[],deleted}。

同步Engine methods（Result<_,String>）：
config()->ConfigBundle，save_config(ConfigBundle)->ConfigBundle，courses(date:String)->Vec<CourseView>，change_course(course_id,subject_id)->()，library(LibraryQuery)->LibraryPage，detail(id)->ArchiveDetail，edit_archive(id,name:String,course_id:Option<String>)->()，favorite_save(FixedFolder)->()，favorite_remove(id)->()，trash(id)->()，trash_list()->Vec<ArchiveDetail>，restore(id)->()，purge(id)->()，busy_volumes()->Vec<String>，thumbnail(id)->Option<String>（PNG data URI；不支持格式返回None），import_timetable(text,format)->Timetable，export_config()->String。

save_config必须验证并事务保存app/capture/timetable/favorites；普通课表编辑不重算历史，当日调课只更改该课快照及记录；事件归属首次有效时间/当天下一节enabled，放学后未分类、不跨午夜。迁移现有SQLite版本1历史；删除移自有recovery区、恢复/永久清理明确动作与中断意图。保持根/来源GUID句柄保护、readonly、导出租约。库只元数据分页、受控Rust regex、打开/导出依旧fresh hash。Engine lease_archive允许所有可用实体，不限当天。导出name从detail元数据取得。

## Native文件模块（fs_jobs/path_guard/devices所有者）

扩展DriveInfo：root,name,identity及deviceId:Option<String>,volumeId:String,identityReliable:bool,isUsb:bool,canEject:bool。DirectoryView扩volumeId、writable（实际访问权限探测），DirectoryEntry增加identity（目录用于拖入）及extension。

新增 FsJobs::browse_options(path,data_root,show_hidden)->DirectoryView；保留browse。preview_copy(sources,request,data_root)->CopyPlan{id,items:Vec<ConflictItem>}; ConflictItem{sourceId,sourceName,targetName,exists,existingSize}。plan_ids(plan_id)->Vec<String>；start_plan(sources,plan_id,resolutions:Vec<ConflictResolution>,data_root)->CopyJob，ConflictResolution{sourceId,policy:keepBoth/skip/replace}。计划有界、短期有效；批准后必须再核对目标及现有文件身份/内容/修改，变化拒绝，不静默覆盖。sources始终Rust-only+corelease交接。

文件基础方法：create_folder(path,identity,name,data_root)->()；rename_entry(path,identity,name,data_root)->()；recycle_entry(path,identity,data_root)->()（不支持系统回收站的USB禁止，不降级永久删除）；watch_directory(path,identity,data_root,callback)->() / unwatch_directory()，回调仅事件，无无限队列。

devices模块：list/enrich驱动器物理关系，格式化卷代际、不能用盘符或磁盘号作永久ID；无可靠硬件身份不套用旧收藏；同物理盘多分区deviceId相同。eject(device_id:String,blocked_volumes:Vec<String>)->()；Windows安全接口只正常请求，不强制dismount；data及活动capture/copy任一volume阻塞。FsJobs::busy_volumes()->Vec<String>。

文件系统安全实现保持句柄固定+GUID路径、同步cancel+join。截图/文件系统编译与测试全部由用户运行。接口实现不要求大规模依赖或wholeSigma移植。
