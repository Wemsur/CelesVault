import type { Ref } from 'vue'
import { useSurfaceGesture } from './surface-gesture'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'

export function useArchiveSurfaceGesture(surface: Ref<HTMLElement | null>) {
	const vault = useVaultStore(); const platform = usePlatform()
	let dragging = false
	return useSurfaceGesture(surface, {
		canHold: target => !!target.closest('[data-archive-id][data-archive-state="available"],[data-course-heading]'),
		hold: (target, point) => {
			const tile = target.closest<HTMLElement>('[data-archive-id][data-archive-state="available"]')
			if (tile) {
				tile.dispatchEvent(new CustomEvent('vault-hold', { detail: point }))
				dragging = vault.dragIds.length > 0
				if (dragging) vault.pointerDrag = { ...point, count: vault.dragIds.length }
			} else target.closest('[data-course-heading]')?.dispatchEvent(new CustomEvent('vault-hold', { detail: point }))
		},
		drag: point => { if (dragging) vault.pointerDrag = { ...point, count: vault.dragIds.length } },
		drop: point => {
			if (!dragging) return
			dragging = false
			const target = document.elementFromPoint(point.x, point.y)?.closest<HTMLElement>('[data-copy-target]')?.dataset.copyTarget
			if (target) void vault.dropAt(platform, target); else vault.endDrag()
		},
		cancel: () => { if (dragging) { dragging = false; vault.endDrag() } },
	})
}
