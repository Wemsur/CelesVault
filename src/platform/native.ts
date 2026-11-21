import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { PlatformClient, RuntimeInfo, WindowState } from './types'
import type { EngineStatus, ArchiveItem, CaptureSettings, DriveInfo, DirectoryView, CopyJob, ConfigBundle, CourseView, LibraryPage, ArchiveDetail, Timetable, CopyPlan, FileRecovery } from './vault-types'

export function createNativeAdapter(): PlatformClient {
	return {
		mode: 'native',
		window: {
			setScale: percent => invoke('set_interface_scale', { percent }),
			getState: () => invoke<WindowState>('window_state'), minimize: () => invoke('minimize_window'),
			toggleMaximize: () => invoke('toggle_maximize_window'), hide: () => invoke('hide_window'),
			onStateChanged: listener => listen<WindowState>('vault:window-state', event => listener(event.payload)),
		},
		app: { getRuntimeInfo: () => invoke<RuntimeInfo>('runtime_info') },
		vault: {
			status: () => invoke<EngineStatus>('vault_status'), chooseRoot: () => invoke<EngineStatus | null>('choose_data_root'),
			today: () => invoke<ArchiveItem[]>('vault_today'), pause: paused => invoke<EngineStatus>('vault_pause', { paused }),
			restartMonitoring: () => invoke<EngineStatus>('restart_monitoring'),
			dismissFailed: taskId => invoke('dismiss_failed_capture', { taskId }),
			retry: taskId => invoke('vault_retry', { taskId }), openArchive: id => invoke('open_archive', { id }),
			getCaptureSettings: () => invoke<CaptureSettings>('capture_settings'), setCaptureSettings: settings => invoke('set_capture_settings', { settings }),
			config: () => invoke<ConfigBundle>('vault_config'), saveConfig: config => invoke<ConfigBundle>('save_vault_config', { config }), saveApp: app => invoke<ConfigBundle>('save_app_settings', { settings: app }),
			onConfigChanged: listener => listen('vault:config-changed', () => listener()),
			courses: date => invoke<CourseView[]>('vault_courses', { date }), changeCourse: (courseId, subjectId) => invoke('change_course', { courseId, subjectId }),
			library: query => invoke<LibraryPage>('library_query', { query }), detail: id => invoke<ArchiveDetail>('archive_detail', { id }),
			editArchive: (id, name, courseId) => invoke('edit_archive', { id, name, courseId }), moveUsage: (id, fromCourseId, toCourseId) => invoke('move_usage', { id, fromCourseId, toCourseId }),
			importConfig: text => invoke<ConfigBundle>('import_config', { text }), exportTimetable: format => invoke<string>('export_timetable', { format }),
			favoriteSave: folder => invoke('save_favorite', { folder }), favoriteRemove: id => invoke('remove_favorite', { id }),
			trash: id => invoke('trash_archive', { id }), trashList: () => invoke<ArchiveDetail[]>('trash_list'),
			restore: id => invoke('restore_archive', { id }), purge: id => invoke('purge_archive', { id }),
			thumbnail: id => invoke<string | null>('archive_thumbnail', { id }), importTimetable: (text, format) => invoke<Timetable>('import_timetable', { text, format }),
			exportConfig: () => invoke<string>('export_config'), saveText: (text, suggestedName) => invoke<boolean>('save_text_export', { text, suggestedName }),
		},
		files: {
			drives: () => invoke<DriveInfo[]>('list_drives'), browse: (path, showHidden = false) => invoke<DirectoryView>('browse_directory', { path, showHidden }),
			export: request => invoke<CopyJob>('export_archives', { request }), previewCopy: request => invoke<CopyPlan>('preview_copy', { request }),
			startPlan: (planId, resolutions) => invoke<CopyJob>('start_copy_plan', { planId, resolutions }),
			jobs: () => invoke<CopyJob[]>('copy_jobs'), cancel: jobId => invoke('cancel_copy', { jobId }),
			createFolder: (path, identity, name) => invoke('create_folder', { path, identity, name }),
			rename: (path, identity, name) => invoke('rename_entry', { path, identity, name }), recycle: (path, identity) => invoke('recycle_entry', { path, identity }),
			open: (path, identity) => invoke('open_browser_entry', { path, identity }), reveal: (path, identity) => invoke('reveal_browser_entry', { path, identity }),
			watch: (path, identity) => invoke('watch_directory', { path, identity }), unwatch: () => invoke('unwatch_directory'),
			onDirectoryChanged: listener => listen('vault:directory-changed', listener), eject: deviceId => invoke('eject_device', { deviceId }),
			recoveries: (path, identity) => invoke<FileRecovery[]>('file_recoveries', { path, identity }), restoreRecovery: (path, identity, recoveryId) => invoke('restore_file_recovery', { path, identity, recoveryId }),
		},
	}
}
