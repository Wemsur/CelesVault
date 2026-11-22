import { PlatformUnavailableError, type PlatformClient, type WindowState } from './types'
import { createPreviewFixtures, DEMO_ROOT } from './preview-fixtures'

export function createPreviewAdapter(): PlatformClient {
	if (!import.meta.env.DEV) throw new PlatformUnavailableError('内存演示仅限开发环境')
	const fixtures = createPreviewFixtures()
	const configListeners = new Set<() => void>()
	const configChanged = () => configListeners.forEach(listener => listener())
	const status = () => ({ dataRoot: DEMO_ROOT, monitorState: 'unconfigured' as const, monitorDetail: '演示数据：仅浏览器内存，不监测、不访问磁盘，刷新全部重置。', pendingCount: fixtures.today().filter(item => item.state === 'pending').length, failedCount: fixtures.today().filter(item => item.state === 'failed').length, health: ['所有课程、磁盘、文件和操作结果均为内存演示，不是Native能力验证。'], hasErrors: false })
	let state: WindowState = { maximized: false, visible: true }
	const listeners = new Set<(state: WindowState) => void>()
	const notify = () => listeners.forEach(listener => listener({ ...state }))
	const unavailable = async (): Promise<never> => { throw new PlatformUnavailableError('浏览器预览不能执行真实文件、监测或持久化操作，请在 Tauri dev 中验证。') }
	return {
		mode: 'preview', demo: true,
		window: {
			async setScale(percent) {
				if (!Number.isInteger(percent) || percent < 75 || percent > 200) throw new Error('应用缩放应为75–200%的整数')
				document.documentElement.style.setProperty('zoom', String(percent / 100))
				document.documentElement.style.setProperty('--vault-preview-scale', String(percent / 100))
				document.documentElement.classList.add('vault-preview-scaled')
			},
			async getState() { return { ...state } },
			async minimize() { window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最小化，浏览器窗口不变。' })) },
			async toggleMaximize() { state = { ...state, maximized: !state.maximized }; notify(); window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '已模拟最大化状态，不是系统窗口操作。' })) },
			async hide() { window.dispatchEvent(new CustomEvent('vault:preview-window-action', { detail: '浏览器没有系统托盘，页面保持打开；未执行系统隐藏。' })) },
			async onStateChanged(listener) { listeners.add(listener); return () => { listeners.delete(listener) } },
		},
		vault: {
			async status() { return status() },
			chooseRoot: unavailable, async today() { return fixtures.today() }, pause: unavailable, restartMonitoring: unavailable,
			async retry(taskId) { fixtures.retry(taskId) }, async dismissFailed(taskId) { fixtures.dismiss(taskId) }, openArchive: unavailable,
			async getCaptureSettings() { return fixtures.config().capture },
			async setCaptureSettings(capture) { const value=fixtures.config();value.capture=capture;fixtures.saveConfig(value);configChanged() },
			async config() { return fixtures.config() }, async saveConfig(value) { const saved=fixtures.saveConfig(value);configChanged();return saved },
			async saveApp(app) { const value=fixtures.config();value.app=app;const saved=fixtures.saveConfig(value);configChanged();return saved },
			async onConfigChanged(listener) { configListeners.add(listener);return () => { configListeners.delete(listener) } },
			async courses() { return fixtures.courses() }, async changeCourse(id,subjectId) { fixtures.changeCourse(id,subjectId) },
			async library(query) { return fixtures.library(query) }, async detail(id) { return fixtures.detail(id) }, editArchive: unavailable, moveUsage: unavailable, importConfig: unavailable, exportTimetable: unavailable,
			async favoriteSave(folder) { fixtures.favoriteSave(folder);configChanged() }, async favoriteRemove(id) { fixtures.favoriteRemove(id);configChanged() },
			async trash(id) { fixtures.trash(id) }, async trashList() { return fixtures.trashList() }, async restore(id) { fixtures.restore(id) }, async purge(id) { fixtures.purge(id) },
			async thumbnail() { return null }, importTimetable: unavailable, exportConfig: unavailable, saveText: unavailable,
		},
		files: {
			async drives() { return fixtures.drives() }, async browse(path) { return fixtures.browse(path) },
			async export(request) { const plan=fixtures.previewCopy(request.ids,request.targetPath,request.targetIdentity);return fixtures.startPlan(plan.id,[]) },
			async previewCopy(request) { return fixtures.previewCopy(request.ids,request.targetPath,request.targetIdentity) }, async startPlan(id,resolutions) { return fixtures.startPlan(id,resolutions) },
			async jobs() { return fixtures.jobs() }, cancel: unavailable, createFolder: unavailable, rename: unavailable, recycle: unavailable, open: unavailable, reveal: unavailable,
			async watch() {}, async unwatch() {}, async onDirectoryChanged() { return () => {} }, eject: unavailable, async recoveries() { return [] }, restoreRecovery: unavailable,
		},
		app: { async getRuntimeInfo() { return {
			mode: 'preview', version: '0.2.0', elevated: false, defaultDataRoot: DEMO_ROOT, monitorState: 'unavailable',
			monitorDetail: '内存演示：课程、文件、磁盘、固定和复制结果均为虚构；刷新重置，不读取 Windows 文件。',
			capabilities: { capture: false, archive: false, fileBrowser: false, copy: false, eject: false, persistentSettings: false },
		} } },
	}
}
