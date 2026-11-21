import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { PlatformClient } from '@/platform'
import type { LibraryPage, LibraryQuery } from '@/platform/vault-types'

export const useLibraryStore = defineStore('library', () => {
	const query = ref<LibraryQuery>({ name: '', regex: false, subjects: [], dateFrom: null, dateTo: null, extensions: [], groupBy: 'none', sortBy: 'archived', descending: true, offset: 0, limit: 100 })
	const result = ref<LibraryPage>({ rows: [], total: 0 })
	const loading = ref(false)
	const error = ref('')
	const listView = ref(false)
	let request = 0
	async function load(platform: PlatformClient) {
		const token = ++request
		loading.value = true
		error.value = ''
		try {
			const page = await platform.vault.library(JSON.parse(JSON.stringify(query.value)))
			if (token === request) result.value = page
		} catch (failure) {
			if (token === request) { error.value = String(failure); result.value = { rows: [], total: 0 } }
		} finally { if (token === request) loading.value = false }
	}
	function search(name: string, regex: boolean) { query.value.name = name; query.value.regex = regex; query.value.offset = 0 }
	function cancelPending() { request++; loading.value = false }
	return { query, result, loading, error, listView, load, search, cancelPending }
})
