import { reactive } from 'vue'

/** Session-only counters: no filename, path, coordinates or telemetry. */
export const touchDiagnostics = reactive({
	started: 0, scrolls: 0, holds: 0, cancelled: 0, scrolledPixels: 0, mouseStarts: 0,
	maxTouchPoints: navigator.maxTouchPoints,
	lastPointer: '尚未收到', lastSurface: '无', lastResult: '无',
})
export function surfaceName(root: HTMLElement): string {
	if (root.matches('.vault-courses')) return root.classList.contains('horizontal') ? '横向课程' : '纵向课程'
	if (root.matches('.vault-library-records')) return '资料库'
	if (root.matches('.vault-directory-list')) return '目录'
	if (root.matches('.vault-fixed-folders')) return '固定夹'
	return '滚动区'
}
