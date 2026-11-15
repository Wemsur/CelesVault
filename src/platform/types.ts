import type { EngineStatus, ArchiveItem, CaptureSettings, DriveInfo, DirectoryView, ExportRequest, CopyJob } from './vault-types'
export type Unlisten = () => void
export type HomeDirection = 'horizontal' | 'vertical'
export interface WindowState { maximized: boolean; visible: boolean }
export interface Capabilities {
	capture: boolean
	archive: boolean
	fileBrowser: boolean
	copy: boolean
	eject: boolean
	persistentSettings: boolean
}
export interface RuntimeInfo {
	mode: 'native' | 'preview'
	version: string
	elevated: boolean
	defaultDataRoot: string
	monitorState: 'unavailable' | 'paused' | 'running' | 'error'
	monitorDetail: string
	capabilities: Capabilities
}
export interface PlatformClient {
	readonly mode: 'native' | 'preview'
	window: {
		getState(): Promise<WindowState>
		minimize(): Promise<void>
		toggleMaximize(): Promise<void>
		hide(): Promise<void>
		onStateChanged(listener: (state: WindowState) => void): Promise<Unlisten>
	}
	app: { getRuntimeInfo(): Promise<RuntimeInfo> }
	vault: {
		status(): Promise<EngineStatus>
		chooseRoot(): Promise<EngineStatus | null>
		today(): Promise<ArchiveItem[]>
		pause(paused: boolean): Promise<EngineStatus>
		retry(taskId: string): Promise<void>
		openArchive(id: string): Promise<void>
		getCaptureSettings(): Promise<CaptureSettings>
		setCaptureSettings(settings: CaptureSettings): Promise<void>
	}
	files: {
		drives(): Promise<DriveInfo[]>
		browse(path: string): Promise<DirectoryView>
		export(request: ExportRequest): Promise<CopyJob>
		jobs(): Promise<CopyJob[]>
		cancel(jobId: string): Promise<void>
	}
}

export class PlatformUnavailableError extends Error {
	constructor(message: string) { super(message); this.name = 'PlatformUnavailableError' }
}
