import { ref } from 'vue'
import type { PlatformClient } from './types'

export const previewScale = ref<number | null>(null)
let queue = Promise.resolve()
let requested = 0
/** WebView zoom scales px, rem, fixed controls and body-teleported UI together. */
export function applyInterfaceScale(platform: PlatformClient, percent: number): Promise<void> {
	if (!Number.isInteger(percent) || percent < 75 || percent > 200) return Promise.reject(new Error('整个应用缩放应为75–200%的整数'))
	const token = ++requested
	const operation = queue.catch(() => {}).then(async () => {
		if (token !== requested) return
		document.documentElement.style.fontSize = '16px'
		await platform.window.setScale(percent)
	})
	queue = operation
	return operation
}
