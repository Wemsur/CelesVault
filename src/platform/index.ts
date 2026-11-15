import { inject, type InjectionKey } from 'vue'
import type { PlatformClient } from './types'
import { PlatformUnavailableError } from './types'

export const platformKey: InjectionKey<PlatformClient> = Symbol('CelesVaultPlatform')
export function usePlatform(): PlatformClient {
	const platform = inject(platformKey)
	if (!platform) throw new PlatformUnavailableError('未初始化平台接口')
	return platform
}

export async function createPlatform(): Promise<PlatformClient> {
	const native = '__TAURI_INTERNALS__' in window
	if (native) {
		const { createNativeAdapter } = await import('./native')
		return createNativeAdapter()
	}
	if (!import.meta.env.DEV) {
		throw new PlatformUnavailableError('正式应用需要 Tauri 宿主，不能静默切换到浏览器测试模式。')
	}
	const { createPreviewAdapter } = await import('./preview')
	return createPreviewAdapter()
}

export type { PlatformClient, RuntimeInfo, HomeDirection, WindowState, Unlisten } from './types'
