import { PlatformUnavailableError, type PlatformClient, type WindowState } from './types'

export function createPreviewAdapter(): PlatformClient {
	let state: WindowState = { maximized: false, visible: true }
	const listeners = new Set<(state: WindowState) => void>()
	const notify = () => listeners.forEach(listener => listener({ ...state }))
	const unavailable = async (): Promise<never> => { throw new PlatformUnavailableError('浏览器预览不能执行真实文件、监测或持久化操作，请在 Tauri dev 中验证。') }
	return {
		mode: 'preview',
		window: {
			async getState() { return { ...state } },
			async minimize() {
				window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最小化；浏览器窗口不会最小化。' }))
			},
			async toggleMaximize() {
				state = { ...state, maximized: !state.maximized }; notify()
				window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最大化状态；浏览器窗口尺寸不变。' }))
			},
			async hide() {
				window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟关闭到托盘；浏览器没有系统托盘，页面保持打开。' }))
			},
			async onStateChanged(listener) { listeners.add(listener); return () => { listeners.delete(listener) } },
		},
		vault: {
			async status() { return { dataRoot: null, monitorState: 'unconfigured', monitorDetail: '浏览器预览不监测、不读取本机文件。', pendingCount: 0, failedCount: 0, health: [], hasErrors: false } },
			chooseRoot: unavailable, async today() { return [] }, pause: unavailable, retry: unavailable, openArchive: unavailable,
			getCaptureSettings: unavailable, setCaptureSettings: unavailable,
		},
		files: { async drives() { return [] }, browse: unavailable, export: unavailable, async jobs() { return [] }, cancel: unavailable },
		app: {
			async getRuntimeInfo() {
				return {
					mode: 'preview', version: '0.2.0', elevated: false,
					defaultDataRoot: '程序目录/data（仅说明，未创建）',
					monitorState: 'unavailable', monitorDetail: '浏览器仅预览界面，不读取 Windows 文件、不监测、不归档、不复制。',
					capabilities: { capture: false, archive: false, fileBrowser: false, copy: false, eject: false, persistentSettings: false },
				}
			},
		},
	}
}
