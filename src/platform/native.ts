import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { PlatformClient, RuntimeInfo, WindowState } from './types'
import type { EngineStatus, ArchiveItem, CaptureSettings, DriveInfo, DirectoryView, CopyJob } from './vault-types'

export function createNativeAdapter(): PlatformClient {
	return {
		mode: 'native',
		window: {
			getState: () => invoke<WindowState>('window_state'),
			minimize: () => invoke('minimize_window'),
			toggleMaximize: () => invoke('toggle_maximize_window'),
			hide: () => invoke('hide_window'),
			onStateChanged: listener => listen<WindowState>('vault:window-state', event => listener(event.payload)),
		},
		app: { getRuntimeInfo: () => invoke<RuntimeInfo>('runtime_info') },
		vault: {
			status: () => invoke<EngineStatus>('vault_status'),
			chooseRoot: () => invoke<EngineStatus | null>('choose_data_root'),
			today: () => invoke<ArchiveItem[]>('vault_today'),
			pause: paused => invoke<EngineStatus>('vault_pause', { paused }),
			retry: taskId => invoke('vault_retry', { taskId }),
			openArchive: id => invoke('open_archive', { id }),
			getCaptureSettings: () => invoke<CaptureSettings>('capture_settings'),
			setCaptureSettings: settings => invoke('set_capture_settings', { settings }),
		},
		files: {
			drives: () => invoke<DriveInfo[]>('list_drives'),
			browse: path => invoke<DirectoryView>('browse_directory', { path }),
			export: request => invoke<CopyJob>('export_archives', { request }),
			jobs: () => invoke<CopyJob[]>('copy_jobs'),
			cancel: jobId => invoke('cancel_copy', { jobId }),
		},
	}
}
