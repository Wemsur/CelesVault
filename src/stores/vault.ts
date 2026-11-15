import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type { PlatformClient } from '@/platform'
import type { EngineStatus, ArchiveItem, DirectoryView, DriveInfo, CopyJob } from '@/platform/vault-types'

export const useVaultStore = defineStore('vault', () => {
	const status = ref<EngineStatus | null>(null)
	const items = ref<ArchiveItem[]>([])
	const drives = ref<DriveInfo[]>([])
	const target = ref<DirectoryView | null>(null)
	const jobs = ref<CopyJob[]>([])
	const selected = ref<string[]>([])
	const busy = ref(false)
	const error = ref('')
	let refreshing = false
	let navigationRequest = 0
	const available = computed(() => items.value.filter(item => item.state === 'available'))
	const selectedAvailable = computed(() => available.value.filter(item => selected.value.includes(item.id)))
	function select(id: string, value: boolean) { selected.value = value ? [...new Set([...selected.value, id])] : selected.value.filter(item => item !== id) }
	async function refresh(platform: PlatformClient) {
		if (refreshing) return
		refreshing = true
		try {
			status.value = await platform.vault.status()
			if (status.value.monitorState === 'error') { target.value = null; selected.value = [] }
			const [today, volumes, currentJobs] = await Promise.all([platform.vault.today(), platform.files.drives(), platform.files.jobs()])
			items.value = today; drives.value = volumes; jobs.value = currentJobs
			selected.value = selected.value.filter(id => today.some(item => item.id === id && item.state === 'available'))
			if (target.value && !volumes.some(volume => target.value!.path.toLowerCase().startsWith(volume.root.toLowerCase()))) target.value = null
			if (target.value && platform.mode === 'native') {
				const expected = target.value
				try { const refreshed = await platform.files.browse(expected.path); if (expected === target.value) target.value = refreshed.identity === expected.identity ? refreshed : null } catch { if (expected === target.value) target.value = null }
			}
		} catch (failure) { error.value = failure instanceof Error ? failure.message : String(failure) }
		finally { refreshing = false }
	}
	async function browse(platform: PlatformClient, path: string) {
		const request = ++navigationRequest
		busy.value = true; error.value = ''; target.value = null
		try { const directory = await platform.files.browse(path); if (request === navigationRequest) target.value = directory }
		catch (failure) { if (request === navigationRequest) { error.value = String(failure); target.value = null } }
		finally { if (request === navigationRequest) busy.value = false }
	}
	async function exportSelected(platform: PlatformClient, conflict: 'keepBoth' | 'skip' = 'keepBoth') {
		if (!target.value || !selectedAvailable.value.length || busy.value) return
		const snapshot = { ids: selectedAvailable.value.map(item => item.id), targetPath: target.value.path, targetIdentity: target.value.identity, conflict }
		busy.value = true; error.value = ''
		try { jobs.value = [await platform.files.export(snapshot), ...jobs.value] }
		catch (failure) { error.value = String(failure) }
		finally { busy.value = false }
	}
	return { status, items, drives, target, jobs, selected, busy, error, available, selectedAvailable, select, refresh, browse, exportSelected }
})
