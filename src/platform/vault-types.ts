export interface EngineStatus {
	dataRoot: string | null
	monitorState: 'unconfigured' | 'running' | 'paused' | 'error'
	monitorDetail: string
	pendingCount: number
	failedCount: number
	health: string[]
	hasErrors: boolean
}
export interface ArchiveItem {
	id: string
	displayName: string
	extension: string
	size: number
	detectedUnixMs: number
	state: 'available' | 'pending' | 'failed'
	taskId: string | null
	error: string | null
	courseId?: string | null
	subjectName?: string | null
}
export interface CaptureSettings {
	mode: 'loose' | 'strict'
	processNames: string[]
	processPaths: string[]
	wpsInstallPaths: string[]
	sources: { path: string; recursive: boolean }[]
	includeDefaultSources: boolean
	includeUsb: boolean
	extensions: string[]
	excludedPaths: string[]
	excludedSubjectIds: string[]
}
export interface Subject { id: string; name: string }
export interface TimeSlot { id: string; label: string; start: string; end: string; isClass: boolean }
export interface WeekLesson { weekday: number; slotId: string; subjectId: string; enabled: boolean }
export interface Timetable { subjects: Subject[]; slots: TimeSlot[]; lessons: WeekLesson[] }
export interface AppSettings {
	homeDirection: 'horizontal' | 'vertical'
	horizontalRatio: number
	verticalRatio: number
	horizontalTargetRatio: number
	verticalTargetRatio: number
	libraryRatio: number
	scalePercent: number
	showHidden: boolean
	browserView: 'grid' | 'list'
	windowWidth: number
	windowHeight: number
	windowMaximized: boolean
	closeNoticed: boolean
	notificationSeconds: number
}
export interface FixedFolder { id: string; deviceId: string; volumeId: string; relativePath: string; alias: string; position: number }
export interface ConfigBundle { version: number; app: AppSettings; capture: CaptureSettings; timetable: Timetable; favorites: FixedFolder[] }
export interface CourseView {
	id: string; date: string; slotId: string | null; subjectId: string | null; subjectName: string; label: string
	startUnixMs: number; endUnixMs: number; enabled: boolean; items: ArchiveItem[]
}
export type GroupBy = 'none' | 'date' | 'month' | 'subject' | 'course' | 'type'
export type SortBy = 'archived' | 'used' | 'name' | 'size'
export interface LibraryQuery {
	name: string; regex: boolean; subjects: string[]; dateFrom: string | null; dateTo: string | null
	extensions: string[]; groupBy: GroupBy; sortBy: SortBy; descending: boolean; offset: number; limit: number
}
export interface LibraryRow { groupKey: string; groupLabel: string; item: ArchiveItem }
export interface LibraryPage { rows: LibraryRow[]; total: number }
export interface UsageDetail { date: string; courseId: string; subjectName: string; detectedUnixMs: number }
export interface ArchiveDetail { item: ArchiveItem; archivePath: string; sourcePaths: string[]; uses: UsageDetail[]; deleted: boolean }
export interface DriveInfo {
	root: string; name: string; identity: string; deviceId: string | null; volumeId: string
	identityReliable: boolean; isUsb: boolean; canEject: boolean
}
export interface DirectoryEntry {
	name: string; path: string; isDir: boolean; size: number; modifiedUnixMs: number; identity: string; extension: string
}
export interface DirectoryView { path: string; identity: string; volumeId: string; writable: boolean; entries: DirectoryEntry[] }
export interface ExportRequest { ids: string[]; targetPath: string; targetIdentity: string; conflict: 'keepBoth' | 'skip' }
export interface ConflictItem { sourceId: string; sourceName: string; targetName: string; exists: boolean; existingSize: number | null }
export type ConflictPolicy = 'keepBoth' | 'skip' | 'replace'
export interface ConflictResolution { sourceId: string; policy: ConflictPolicy }
export interface CopyPlan { id: string; items: ConflictItem[] }
export interface FileRecovery { id: string; kind: string; originalName: string; heldName: string; heldPath: string; canRestore: boolean; error: string | null }
export interface CopyJob { id: string; state: 'running' | 'completed' | 'failed' | 'cancelled'; total: number; processed: number; copied: number; skipped: number; error: string | null }
export const defaultAppSettings = (): AppSettings => ({
	homeDirection: 'horizontal', horizontalRatio: .5, verticalRatio: .5, horizontalTargetRatio: .5, verticalTargetRatio: .5,
	libraryRatio: .5, scalePercent: 100, showHidden: false, browserView: 'grid', windowWidth: 1280, windowHeight: 800,
	windowMaximized: false, closeNoticed: false, notificationSeconds: 6,
})
export const emptyConfig = (): ConfigBundle => ({
	version: 1, app: defaultAppSettings(), timetable: { subjects: [], slots: [], lessons: [] }, favorites: [],
	capture: { mode: 'loose', processNames: ['wps.exe', 'wpp.exe', 'et.exe', 'wpspdf.exe', 'photolaunch.exe'], processPaths: [],
		wpsInstallPaths: [], sources: [], includeDefaultSources: true, includeUsb: true,
		extensions: ['doc', 'docx', 'ppt', 'pptx', 'xls', 'xlsx', 'pdf', 'jpg', 'jpeg', 'png', 'webp'], excludedPaths: [], excludedSubjectIds: [] },
})
export const localDate = (date = new Date()) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
