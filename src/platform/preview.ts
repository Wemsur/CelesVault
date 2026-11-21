import { PlatformUnavailableError, type PlatformClient, type WindowState } from './types'
import { emptyConfig } from './vault-types'

export function createPreviewAdapter(): PlatformClient {
	let state: WindowState = { maximized: false, visible: true }
	const listeners = new Set<(state: WindowState) => void>()
	const notify = () => listeners.forEach(listener => listener({ ...state }))
	const unavailable = async (): Promise<never> => { throw new PlatformUnavailableError('浏览器预览不能执行真实文件、监测或持久化操作，请在 Tauri dev 中验证。') }
	return {
		mode: 'preview',
		window: {
			async getState() { return { ...state } },
			async minimize() { window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最小化，浏览器窗口不变。' })) },
			async toggleMaximize() { state = { ...state, maximized: !state.maximized }; notify(); window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最大化状态，不是系统窗口操作。' })) },
			async hide() { window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '浏览器没有系统托盘，页面保持打开；未执行系统隐藏。' })) },
			async onStateChanged(listener) { listeners.add(listener); return () => { listeners.delete(listener) } },
		},
		vault: {
			async status() { return { dataRoot: null, monitorState: 'unconfigured', monitorDetail: '浏览器预览不监测、不读取本机文件。', pendingCount: 0, failedCount: 0, health: [], hasErrors: false } },
			chooseRoot: unavailable, async today() { return [] }, pause: unavailable, restartMonitoring: unavailable, retry: unavailable, dismissFailed: unavailable, openArchive: unavailable,
			getCaptureSettings: unavailable, setCaptureSettings: unavailable, async config() { return emptyConfig() }, saveConfig: unavailable, saveApp: unavailable,
			async onConfigChanged() { return () => {} },
			async courses() { return [] }, changeCourse: unavailable, async library() { return { rows: [], total: 0 } }, detail: unavailable, editArchive: unavailable, moveUsage: unavailable, importConfig: unavailable, exportTimetable: unavailable,
			favoriteSave: unavailable, favoriteRemove: unavailable, trash: unavailable, async trashList() { return [] }, restore: unavailable, purge: unavailable,
			async thumbnail() { return null }, importTimetable: unavailable, exportConfig: unavailable, saveText: unavailable,
		},
		files: {
			async drives() { return [] }, browse: unavailable, export: unavailable, previewCopy: unavailable, startPlan: unavailable,
			async jobs() { return [] }, cancel: unavailable, createFolder: unavailable, rename: unavailable, recycle: unavailable, open: unavailable, reveal: unavailable,
			watch: unavailable, async unwatch() {}, async onDirectoryChanged() { return () => {} }, eject: unavailable, async recoveries() { return [] }, restoreRecovery: unavailable,
		},
		app: { async getRuntimeInfo() { return {
			mode: 'preview', version: '0.2.0', elevated: false, defaultDataRoot: '程序目录/data（仅说明，未创建）', monitorState: 'unavailable',
			monitorDetail: '浏览器仅预览界面，不读取 Windows 文件、不监测、不归档、不复制。',
			capabilities: { capture: false, archive: false, fileBrowser: false, copy: false, eject: false, persistentSettings: false },
		} } },
	}
}
