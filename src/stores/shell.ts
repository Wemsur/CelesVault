import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { HomeDirection, RuntimeInfo } from '@/platform'

export const useShellStore = defineStore('shell', () => {
	const homeDirection = ref<HomeDirection>('horizontal')
	const scale = ref('100%')
	const search = ref('')
	const searchRegex = ref(false)
	const settingsOpen = ref(false)
	const runtime = ref<RuntimeInfo | null>(null)
	const statusMessage = ref('')
	const firstCloseAcknowledged = ref(false)
	return { homeDirection, scale, search, searchRegex, settingsOpen, runtime, statusMessage, firstCloseAcknowledged }
})
