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
}
export interface CaptureSettings {
	mode: string
	processNames: string[]
	processPaths: string[]
	wpsInstallPaths: string[]
	sources: { path: string; recursive: boolean }[]
	includeDefaultSources: boolean
	includeUsb: boolean
	extensions: string[]
	excludedPaths: string[]
}
export interface DriveInfo { root: string; name: string; identity: string }
export interface DirectoryEntry { name: string; path: string; isDir: boolean; size: number; modifiedUnixMs: number }
export interface DirectoryView { path: string; identity: string; entries: DirectoryEntry[] }
export interface ExportRequest { ids: string[]; targetPath: string; targetIdentity: string; conflict: 'keepBoth' | 'skip' }
export interface CopyJob { id: string; state: 'running' | 'completed' | 'failed' | 'cancelled'; total: number; processed: number; copied: number; skipped: number; error: string | null }
