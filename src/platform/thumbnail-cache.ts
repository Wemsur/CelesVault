import type { PlatformClient } from './types'

const LIMIT = 96
const TTL = 60_000
const cache = new Map<string, { expires: number; result: Promise<string | null> }>()
const queue: Array<() => void> = []
let active = 0

function drain() {
	while (active < 2 && queue.length) {
		active++
		queue.shift()!()
	}
}

export function archiveThumbnail(platform: PlatformClient, id: string): Promise<string | null> {
	const previous = cache.get(id)
	if (previous && previous.expires > Date.now()) return previous.result
	const result = new Promise<string | null>((resolve, reject) => {
		queue.push(() => {
			platform.vault.thumbnail(id).then(resolve, failure => { cache.delete(id); reject(failure) })
				.finally(() => { active--; drain() })
		})
	})
	cache.delete(id)
	cache.set(id, { expires: Date.now() + TTL, result })
	while (cache.size > LIMIT) cache.delete(cache.keys().next().value!)
	drain()
	return result
}
