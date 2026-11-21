import { acceptHMRUpdate, defineStore, getActivePinia } from 'pinia'
import { computed, ref } from 'vue'
import { favoriteLocation, matchingFavorite, type FolderPinSource } from '@/platform/fixed-folders'
import type { PlatformClient } from '@/platform'
import { emptyConfig, localDate, type AppSettings, type ArchiveDetail, type ArchiveItem, type ConfigBundle, type ConflictPolicy, type CopyJob, type CopyPlan, type CourseView, type DirectoryView, type DriveInfo, type EngineStatus, type ExportRequest } from '@/platform/vault-types'

export const useVaultStore = defineStore('vault', () => {
	const status = ref<EngineStatus | null>(null)
	const config = ref<ConfigBundle>(emptyConfig())
	const configReady = ref(false)
	const items = ref<ArchiveItem[]>([])
	const courses = ref<CourseView[]>([])
	const drives = ref<DriveInfo[]>([])
	const homeTarget = ref<DirectoryView | null>(null)
	const libraryTarget = ref<DirectoryView | null>(null)
	const libraryBrowsing = ref(false)
	const targetContext = ref<'home' | 'library'>('home')
	const target = computed({ get: () => targetContext.value === 'home' ? homeTarget.value : libraryBrowsing.value ? libraryTarget.value : null, set: value => { if (targetContext.value === 'home') homeTarget.value = value; else libraryTarget.value = value } })
	const jobs = ref<CopyJob[]>([])
	const selected = ref<string[]>([])
	const selectedItems = ref<Record<string, ArchiveItem>>({})
	const busy = ref(false)
	const error = ref('')
	const plan = ref<CopyPlan | null>(null)
	const policies = ref<Record<string, ConflictPolicy>>({})
	const detail = ref<ArchiveDetail | null>(null)
	const detailOpen = ref(false)
	const archiveRevision = ref(0)
	function archivesChanged() { archiveRevision.value++ }
	const folderDrag = ref<FolderPinSource | null>(null)
	let favoriteQueue = Promise.resolve()
	function pinnedFolder(path: string, volumeId: string) { return matchingFavorite(config.value.favorites, path, volumeId, drives.value) }
	async function pinFolder(platform: PlatformClient, source: FolderPinSource) {
		const snapshot = { ...source }
		const dataRoot = status.value?.dataRoot
		const operation = favoriteQueue.catch(() => {}).then(async () => {
			if (platform.mode !== 'native') throw new Error('浏览器预览不能持久固定文件夹')
			const directory = await platform.files.browse(snapshot.path, config.value.app.showHidden)
			if (directory.identity !== snapshot.identity || directory.volumeId !== snapshot.volumeId) throw new Error('文件夹或设备身份已变化，请刷新后重新固定')
			const volumes = await platform.files.drives()
			const location = favoriteLocation(directory.path, directory.volumeId, volumes)
			const current = await platform.vault.config()
			if (!dataRoot || status.value?.dataRoot !== dataRoot) throw new Error('持久目录已变化，请重新固定文件夹')
			if (matchingFavorite(current.favorites, directory.path, directory.volumeId, volumes)) { config.value.favorites = current.favorites; return }
			const position = Math.max(-1, ...current.favorites.map(folder => folder.position)) + 1
			await platform.vault.favoriteSave({ id: crypto.randomUUID(), deviceId: location.deviceId, volumeId: location.volumeId, relativePath: location.relativePath,
				alias: location.relativePath.split('/').at(-1) || location.drive.name, position })
			config.value.favorites = (await platform.vault.config()).favorites
		})
		favoriteQueue = operation
		return operation
	}
	async function unpinFolder(platform: PlatformClient, id: string) {
		const operation = favoriteQueue.catch(() => {}).then(async () => { await platform.vault.favoriteRemove(id); config.value.favorites = (await platform.vault.config()).favorites })
		favoriteQueue = operation
		return operation
	}
	const dragIds = ref<string[]>([])
	const pointerDrag = ref<{ x: number; y: number; count: number } | null>(null)
	let refreshing = false
	let rootLoaded: string | null = null
	let navigationRequest = 0
	let appSaveTimer: ReturnType<typeof setTimeout> | undefined
	let saving = Promise.resolve()
	const available = computed(() => items.value.filter(item => item.state === 'available'))
	const selectedAvailable = computed(() => selected.value.map(id => selectedItems.value[id]).filter((item): item is ArchiveItem => Boolean(item && item.state === 'available')))
	function select(item: ArchiveItem, value: boolean) {
		if (item.state !== 'available') return
		selectedItems.value[item.id] = item
		selected.value = value ? [...new Set([...selected.value, item.id])] : selected.value.filter(id => id !== item.id)
	}
	function clearSelection() { selected.value = []; selectedItems.value = {} }
	function selectMany(group: ArchiveItem[], value: boolean) { for (const item of group) select(item, value) }
	async function loadConfig(platform: PlatformClient) { config.value = await platform.vault.config(); configReady.value = true }
	async function refresh(platform: PlatformClient) {
		if (refreshing) return
		refreshing = true
		try {
			status.value = await platform.vault.status()
			if (status.value.dataRoot !== rootLoaded || (!configReady.value && platform.mode === 'preview')) {
				clearSelection(); folderDrag.value = null; homeTarget.value = null; libraryTarget.value = null; configReady.value = false
				if (status.value.dataRoot || platform.mode === 'preview') await loadConfig(platform)
				rootLoaded = status.value.dataRoot
			}
			const [today, volumes, currentJobs, dayCourses] = await Promise.all([platform.vault.today(), platform.files.drives(), platform.files.jobs(), platform.vault.courses(localDate())])
			items.value = today; drives.value = volumes; jobs.value = currentJobs; courses.value = dayCourses
			for (const item of today) if (selectedItems.value[item.id]) selectedItems.value[item.id] = item
			selected.value = selected.value.filter(id => selectedItems.value[id]?.state === 'available')
			for (const context of ['home', 'library'] as const) {
				const previous = context === 'home' ? homeTarget.value : libraryTarget.value
				if (!previous) continue
				const set = (value: DirectoryView | null) => { if (context === 'home') homeTarget.value = value; else libraryTarget.value = value }
				if (!volumes.some(volume => volume.volumeId === previous.volumeId)) { set(null); continue }
				if (platform.mode === 'native') {
					try {
						const updated = await platform.files.browse(previous.path, config.value.app.showHidden)
						const current = context === 'home' ? homeTarget.value : libraryTarget.value
						if (current === previous) set(updated.identity === previous.identity ? updated : null)
					} catch { if ((context === 'home' ? homeTarget.value : libraryTarget.value) === previous) set(null) }
				}
			}
		} catch (failure) { error.value = String(failure) }
		finally { refreshing = false }
	}
	async function browse(platform: PlatformClient, path: string, context = targetContext.value) {
		const request = ++navigationRequest
		busy.value = true; error.value = ''
		try {
			const directory = await platform.files.browse(path, config.value.app.showHidden)
			if (request !== navigationRequest) return
			if (context === 'home') homeTarget.value = directory; else { libraryTarget.value = directory; libraryBrowsing.value = true }
			if (platform.mode === 'native') await platform.files.watch(directory.path, directory.identity)
		} catch (failure) { if (request === navigationRequest) { error.value = String(failure); if (context === 'home') homeTarget.value = null; else libraryTarget.value = null } }
		finally { if (request === navigationRequest) busy.value = false }
	}
	function persistApp(platform: PlatformClient, patch: Partial<AppSettings>) {
		config.value.app = { ...config.value.app, ...patch }
		if (appSaveTimer) clearTimeout(appSaveTimer)
		if (platform.mode !== 'native' || !status.value?.dataRoot) return
		appSaveTimer = setTimeout(() => {
			const desired = JSON.parse(JSON.stringify(config.value.app)) as AppSettings
			saving = saving.then(async () => {
				const saved = await platform.vault.saveApp(desired)
				config.value.capture = saved.capture; config.value.timetable = saved.timetable; config.value.favorites = saved.favorites
			}).catch(failure => { error.value = `界面状态保存失败：${String(failure)}` })
		}, 500)
	}
	async function saveConfig(platform: PlatformClient, value: ConfigBundle) {
		if (appSaveTimer) clearTimeout(appSaveTimer)
		await saving
		config.value = await platform.vault.saveConfig(value)
		configReady.value = true
	}
	async function requestExport(platform: PlatformClient, ids: string[], destination: DirectoryView | null = target.value) {
		if (!destination || !destination.writable || !ids.length || busy.value || platform.mode !== 'native') return
		const request: ExportRequest = { ids: [...new Set(ids)], targetPath: destination.path, targetIdentity: destination.identity, conflict: 'keepBoth' }
		busy.value = true; error.value = ''
		try {
			const preview = await platform.files.previewCopy(request)
			if (preview.items.some(item => item.exists)) {
				plan.value = preview; policies.value = Object.fromEntries(preview.items.map(item => [item.sourceId, 'keepBoth']))
			} else jobs.value = [await platform.files.startPlan(preview.id, []), ...jobs.value]
		} catch (failure) { error.value = String(failure) }
		finally { busy.value = false }
	}
	async function commitPlan(platform: PlatformClient) {
		if (!plan.value || busy.value) return
		busy.value = true; error.value = ''
		try { jobs.value = [await platform.files.startPlan(plan.value.id, Object.entries(policies.value).map(([sourceId, policy]) => ({ sourceId, policy }))), ...jobs.value]; plan.value = null }
		catch (failure) { error.value = String(failure) }
		finally { busy.value = false }
	}
	async function showDetail(platform: PlatformClient, id: string) {
		try { detail.value = await platform.vault.detail(id); detailOpen.value = true } catch (failure) { error.value = String(failure) }
	}
	async function openArchive(platform: PlatformClient, id: string) { try { await platform.vault.openArchive(id) } catch (failure) { error.value = String(failure) } }
	function beginDrag(item: ArchiveItem) {
		if (!selected.value.includes(item.id)) { clearSelection(); select(item, true) }
		dragIds.value = [...selected.value]
	}
	function endDrag() { dragIds.value = []; pointerDrag.value = null }
	async function dropAt(platform: PlatformClient, path: string) {
		const ids = [...dragIds.value]; endDrag()
		if (!ids.length || platform.mode !== 'native') return
		try { await requestExport(platform, ids, await platform.files.browse(path, config.value.app.showHidden)) } catch (failure) { error.value = String(failure) }
	}
	return { status, config, configReady, items, courses, drives, homeTarget, libraryTarget, libraryBrowsing, targetContext, target, jobs, selected, selectedItems, busy, error,
		archiveRevision, archivesChanged, plan, policies, detail, detailOpen, folderDrag, pinnedFolder, pinFolder, unpinFolder, dragIds, pointerDrag, available, selectedAvailable, select, selectMany, clearSelection, refresh, loadConfig, browse, persistApp, saveConfig,
		requestExport, commitPlan, showDetail, openArchive, beginDrag, endDrag, dropAt }
})

if (import.meta.hot) {
	import.meta.hot.accept(acceptHMRUpdate(useVaultStore, import.meta.hot))
	// The previous module had no HMR handler: already-open development windows
	// may retain a pre-folder-pinning store even when their components update.
	const pinia = getActivePinia()
	if (pinia) {
		const existing = useVaultStore(pinia)
		if (typeof existing.pinFolder !== 'function' || typeof existing.unpinFolder !== 'function' || typeof existing.pinnedFolder !== 'function') {
			window.location.reload()
		}
	}
}
