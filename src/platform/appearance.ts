import { computed, ref, watch } from 'vue'
import { useVaultStore } from '@/stores/vault'
import type { AppearanceSettings } from './appearance-types'

const preview = ref<AppearanceSettings | null>(null)
const systemDark = ref(matchMedia('(prefers-color-scheme: dark)').matches)
const systemQuery = matchMedia('(prefers-color-scheme: dark)')
const systemChanged = (event: MediaQueryListEvent) => { systemDark.value = event.matches }
export function useAppearance() {
	const vault = useVaultStore()
	const settings = computed(() => preview.value ?? vault.config.app.appearance)
	const dark = computed(() => settings.value.theme === 'customdark' || (settings.value.theme === 'system' && systemDark.value))
	return { settings, dark, preview }
}
export function initializeAppearance() {
	const { settings, dark } = useAppearance()
	systemDark.value = systemQuery.matches
	systemQuery.addEventListener('change', systemChanged)
	const stop = watch([() => settings.value.hue, dark], ([hue, isDark]) => {
		const html = document.documentElement
		html.classList.remove('dark-mode','light-mode','customdark-mode','customlight-mode')
		html.classList.add(isDark ? 'customdark-mode' : 'customlight-mode')
		html.style.setProperty('--brand-hue', String(hue))
		html.style.colorScheme = isDark ? 'dark' : 'light'
		document.querySelector('meta[name="color-scheme"]')?.setAttribute('content', isDark ? 'dark' : 'light')
	}, { immediate: true })
	return () => { stop(); systemQuery.removeEventListener('change', systemChanged) }
}

/** Only a bounded display copy is persisted; never retain an external image path. */
export async function prepareBackgroundImage(file: File): Promise<string> {
	if (!['image/png','image/jpeg','image/webp'].includes(file.type)) throw new Error('请选择 PNG、JPEG 或 WebP 图片')
	if (file.size > 12 * 1024 * 1024) throw new Error('原图片不能超过12MiB')
	const bitmap = await createImageBitmap(file)
	try {
		if (bitmap.width * bitmap.height > 32_000_000 || Math.max(bitmap.width, bitmap.height) > 16_384) throw new Error('图片尺寸过大，请先缩小图片')
		const ratio = Math.min(1, 1600 / Math.max(bitmap.width, bitmap.height))
		const canvas = document.createElement('canvas')
		canvas.width = Math.max(1, Math.round(bitmap.width * ratio)); canvas.height = Math.max(1, Math.round(bitmap.height * ratio))
		const context = canvas.getContext('2d')
		if (!context) throw new Error('图片转换不可用')
		context.fillStyle = '#eeeeee'; context.fillRect(0,0,canvas.width,canvas.height)
		context.drawImage(bitmap,0,0,canvas.width,canvas.height)
		for (const quality of [.85,.7,.55,.4,.25]) {
			const url = canvas.toDataURL('image/jpeg', quality)
			if (url.length <= 436_900) return url
		}
		throw new Error('图片转换后仍超过320KiB，请选择较简单的图片')
	} finally { bitmap.close() }
}
