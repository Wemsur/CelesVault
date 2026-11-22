import type { EngineStatus, ArchiveItem, CaptureSettings, DriveInfo, DirectoryView, ExportRequest, CopyJob, ConfigBundle, CourseView, LibraryQuery, LibraryPage, ArchiveDetail, Timetable, FixedFolder, CopyPlan, ConflictResolution, FileRecovery } from './vault-types'
export type Unlisten = () => void
export type HomeDirection = 'horizontal' | 'vertical'
export interface WindowState { maximized: boolean; visible: boolean }
export interface Capabilities { capture: boolean; archive: boolean; fileBrowser: boolean; copy: boolean; eject: boolean; persistentSettings: boolean }
export interface RuntimeInfo {
	mode: 'native' | 'preview'; version: string; elevated: boolean; defaultDataRoot: string
	monitorState: 'unavailable' | 'paused' | 'running' | 'error'; monitorDetail: string; capabilities: Capabilities
}
export interface PlatformClient {
	readonly mode: 'native' | 'preview'
	/** Only the DEV browser adapter provides in-memory interaction fixtures. */
	readonly demo?: boolean
	window: { setScale(percent: number): Promise<void>; getState(): Promise<WindowState>; minimize(): Promise<void>; toggleMaximize(): Promise<void>; hide(): Promise<void>; onStateChanged(listener: (state: WindowState) => void): Promise<Unlisten> }
	app: { getRuntimeInfo(): Promise<RuntimeInfo> }
	vault: {
		status(): Promise<EngineStatus>; chooseRoot(): Promise<EngineStatus | null>; today(): Promise<ArchiveItem[]>
		pause(paused: boolean): Promise<EngineStatus>; restartMonitoring(): Promise<EngineStatus>; retry(taskId: string): Promise<void>; dismissFailed(taskId: string): Promise<void>; openArchive(id: string): Promise<void>
		getCaptureSettings(): Promise<CaptureSettings>; setCaptureSettings(settings: CaptureSettings): Promise<void>
		config(): Promise<ConfigBundle>; saveConfig(config: ConfigBundle): Promise<ConfigBundle>; saveApp(app: ConfigBundle['app']): Promise<ConfigBundle>
		onConfigChanged(listener: () => void): Promise<Unlisten>
		courses(date: string): Promise<CourseView[]>; changeCourse(courseId: string, subjectId: string): Promise<void>
		library(query: LibraryQuery): Promise<LibraryPage>; detail(id: string): Promise<ArchiveDetail>
		editArchive(id: string, name: string, courseId: string | null): Promise<void>; moveUsage(id: string, fromCourseId: string, toCourseId: string): Promise<void>
		importConfig(text: string): Promise<ConfigBundle>; exportTimetable(format: 'csv' | 'json'): Promise<string>
		favoriteSave(folder: FixedFolder): Promise<void>; favoriteRemove(id: string): Promise<void>
		trash(id: string): Promise<void>; trashList(): Promise<ArchiveDetail[]>; restore(id: string): Promise<void>; purge(id: string): Promise<void>
		thumbnail(id: string): Promise<string | null>; importTimetable(text: string, format: 'csv' | 'json'): Promise<Timetable>
		exportConfig(): Promise<string>; saveText(text: string, suggestedName: string): Promise<boolean>
	}
	files: {
		drives(): Promise<DriveInfo[]>; browse(path: string, showHidden?: boolean): Promise<DirectoryView>
		export(request: ExportRequest): Promise<CopyJob>; previewCopy(request: ExportRequest): Promise<CopyPlan>
		startPlan(planId: string, resolutions: ConflictResolution[]): Promise<CopyJob>
		jobs(): Promise<CopyJob[]>; cancel(jobId: string): Promise<void>
		createFolder(path: string, identity: string, name: string): Promise<void>
		rename(path: string, identity: string, name: string): Promise<void>
		recycle(path: string, identity: string): Promise<void>
		open(path: string, identity: string): Promise<void>; reveal(path: string, identity: string): Promise<void>
		watch(path: string, identity: string): Promise<void>; unwatch(): Promise<void>
		onDirectoryChanged(listener: () => void): Promise<Unlisten>; eject(deviceId: string): Promise<void>
		recoveries(path: string, identity: string): Promise<FileRecovery[]>; restoreRecovery(path: string, identity: string, recoveryId: string): Promise<void>
	}
}
export class PlatformUnavailableError extends Error {
	constructor(message: string) { super(message); this.name = 'PlatformUnavailableError' }
}
