import type { PlatformClient } from './types'

/** The native client never sets demo; production browser mode is rejected at boot. */
export function allowsDemoInteractions(platform: PlatformClient): boolean {
	return platform.mode === 'native' || (import.meta.env.DEV && platform.mode === 'preview' && platform.demo === true)
}
export function interactionId(platform: PlatformClient): string {
	// randomUUID requires HTTPS on LAN phones. Demo IDs never identify real data.
	if (import.meta.env.DEV && platform.mode === 'preview' && platform.demo) return `demo-${Date.now().toString(36)}-${Math.random().toString(36).slice(2,12)}`
	return crypto.randomUUID()
}
