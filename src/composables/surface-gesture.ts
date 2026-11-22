import { onDeactivated, onMounted, onUnmounted, watch, type Ref } from 'vue'
import { surfaceName, touchDiagnostics } from './touch-diagnostics'

type Point = { x: number; y: number }
interface SurfaceGestureOptions {
	/** File/heading adapters opt into holding; controls are always tap/scroll only. */
	canHold?: (target: HTMLElement) => boolean
	/** Directory entries are buttons, but the whole entry may be held to pin. */
	holdFromControl?: boolean
	hold?: (target: HTMLElement, point: Point) => void
	drag?: (point: Point) => void
	drop?: (point: Point) => void
	cancel?: () => void
}
interface Gesture {
	id: number; target: HTMLElement; root: HTMLElement; start: Point; previous: Point
	axis: 'x' | 'y' | null; moved: boolean; rootMoved: boolean; held: boolean; snap: string; snapPriority: string
}
const INTERACTIVE = 'button,input,textarea,select,a,[role="checkbox"],[role="slider"],[role="separator"],[contenteditable="true"]'
const owners = new Map<number, HTMLElement>()

/** Exactly one pointer owner per scroll surface. No simultaneous TouchEvent path.
 * touch-action:none on the surface leaves short pans and holds to this controller.
 * Events on buttons still enter here in capture phase; a tap remains a native click.
 */
export function useSurfaceGesture(surface: Ref<HTMLElement | null>, options: SurfaceGestureOptions = {}) {
	let gesture: Gesture | null = null
	let holdTimer: ReturnType<typeof setTimeout> | undefined
	let suppressedUntil = 0
	let suppressAllClicks = false
	let observed: HTMLElement | null = null
	function clearTimer() { if (holdTimer) clearTimeout(holdTimer); holdTimer = undefined }
	function restoreSnap(state: Gesture) {
		if (state.snap) state.root.style.setProperty('scroll-snap-type', state.snap, state.snapPriority)
		else state.root.style.removeProperty('scroll-snap-type')
	}
	function snapCourse(root: HTMLElement) {
		if (!root.matches('.vault-courses') || !root.isConnected) return
		const horizontal = root.classList.contains('horizontal')
		const position = horizontal ? root.scrollLeft : root.scrollTop
		const maximum = Math.max(0, horizontal ? root.scrollWidth - root.clientWidth : root.scrollHeight - root.clientHeight)
		let destination = position
		if (position <= 2) destination = 0
		else if (position >= maximum - 2) destination = maximum
		else {
			const bounds = root.getBoundingClientRect()
			const factor = (horizontal ? bounds.width / root.clientWidth : bounds.height / root.clientHeight) || 1
			let distance = Infinity
			const cards = Array.from(root.querySelectorAll<HTMLElement>('[data-course-id]'))
			for (const [index, card] of cards.entries()) {
				const rect = card.getBoundingClientRect()
				const offset = (horizontal ? rect.left - bounds.left : rect.top - bounds.top) / factor
				const size = (horizontal ? rect.width : rect.height) / factor
				const viewport = horizontal ? root.clientWidth : root.clientHeight
				const point = index === 0 ? 0 : index === cards.length - 1 ? maximum : Math.min(maximum, Math.max(0, position + offset + (size - viewport) / 2))
				if (Math.abs(point - position) < distance) { distance = Math.abs(point - position); destination = point }
			}
		}
		root.scrollTo(horizontal ? { left: destination, behavior: 'instant' } : { top: destination, behavior: 'instant' })
	}
	function finish(cancelled: boolean, point?: Point) {
		clearTimer()
		const state = gesture
		if (!state) return
		gesture = null; owners.delete(state.id)
		if (cancelled) touchDiagnostics.cancelled++
		else if (state.moved) touchDiagnostics.scrolls++
		touchDiagnostics.lastResult = cancelled ? '取消' : state.held ? '长按结束' : state.moved ? '滑动结束' : '轻点'
		window.removeEventListener('pointermove', move, true)
		window.removeEventListener('pointerup', up, true)
		window.removeEventListener('pointercancel', pointerCancel, true)
		window.removeEventListener('pointerdown', otherPointer, true)
		window.removeEventListener('blur', cancel)
		document.removeEventListener('visibilitychange', visibility)
		if (state.moved || state.held) { suppressedUntil = Date.now() + 800; suppressAllClicks = true }
		if (state.root.hasPointerCapture(state.id)) state.root.releasePointerCapture(state.id)
		// Snap once after scrolling, before restoring CSS snapping. Never snap on cancel.
		if (!cancelled && state.rootMoved && !state.held) snapCourse(state.root)
		restoreSnap(state)
		if (state.held) {
			if (!cancelled && point) options.drop?.(point)
			else options.cancel?.()
		}
	}
	function cancel() { finish(true) }
	function pointerCancel(event: PointerEvent) { if (gesture?.id === event.pointerId) cancel() }
	function visibility() { if (document.hidden) cancel() }
	function otherPointer(event: PointerEvent) { if (gesture && event.pointerId !== gesture.id) cancel() }
	function down(event: PointerEvent) {
		if (event.pointerType === 'mouse') { touchDiagnostics.mouseStarts++; return }
		if (event.pointerType !== 'touch' && event.pointerType !== 'pen') return
		if (!event.isPrimary) { cancel(); return }
		if (event.button !== 0 || owners.has(event.pointerId) || !surface.value) return
		cancel(); suppressAllClicks = false
		const target = event.target instanceof Element ? event.target.closest<HTMLElement>('*') : null
		if (!target) return
		const root = surface.value
		owners.set(event.pointerId, root)
		touchDiagnostics.started++; touchDiagnostics.lastPointer = event.pointerType; touchDiagnostics.lastSurface = surfaceName(root)
		gesture = { id: event.pointerId, target, root, start: { x: event.clientX, y: event.clientY }, previous: { x: event.clientX, y: event.clientY }, axis: null, moved: false, rootMoved: false, held: false,
			snap: root.style.getPropertyValue('scroll-snap-type'), snapPriority: root.style.getPropertyPriority('scroll-snap-type') }
		window.addEventListener('pointermove', move, { capture: true, passive: false })
		window.addEventListener('pointerup', up, true)
		window.addEventListener('pointercancel', pointerCancel, true)
		window.addEventListener('pointerdown', otherPointer, true)
		window.addEventListener('blur', cancel)
		document.addEventListener('visibilitychange', visibility)
		if ((options.holdFromControl || !target.closest(INTERACTIVE)) && options.canHold?.(target)) {
			holdTimer = setTimeout(() => {
				const state = gesture
				if (!state || state.moved) return
				if (!state.root.isConnected || !state.target.isConnected) { cancel(); return }
				state.held = true
				touchDiagnostics.holds++
				suppressedUntil = Date.now() + 800; suppressAllClicks = true
				state.root.setPointerCapture(state.id)
				options.hold?.(state.target, state.previous)
			}, 450)
		}
	}
	function scrollElement(node: HTMLElement, axis: 'x' | 'y', delta: number) {
		const before = axis === 'x' ? node.scrollLeft : node.scrollTop
		if (axis === 'x') node.scrollLeft += delta; else node.scrollTop += delta
		const consumed = (axis === 'x' ? node.scrollLeft : node.scrollTop) - before
		touchDiagnostics.scrolledPixels += Math.abs(consumed)
		return delta - consumed
	}
	function move(event: PointerEvent) {
		const state = gesture
		if (!state || state.id !== event.pointerId) return
		if (!state.target.isConnected || !state.root.isConnected) { cancel(); return }
		const point = { x: event.clientX, y: event.clientY }
		if (state.held && (point.x === state.previous.x && point.y === state.previous.y)) return
		if (!state.moved && !state.held) {
			const dx = point.x - state.start.x, dy = point.y - state.start.y
			if (Math.hypot(dx, dy) < 8) return
			clearTimer(); state.moved = true; state.axis = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y'
			state.root.style.setProperty('scroll-snap-type', 'none', 'important')
			state.root.setPointerCapture(state.id)
		}
		if (event.cancelable) event.preventDefault()
		if (state.held) options.drag?.(point)
		else {
			const bounds = state.root.getBoundingClientRect()
			const xScale = bounds.width / state.root.clientWidth || 1, yScale = bounds.height / state.root.clientHeight || 1
			const dx = (state.previous.x - point.x) / xScale, dy = (state.previous.y - point.y) / yScale
			const files = state.target.closest<HTMLElement>('.vault-course-files')
			const horizontal = state.root.matches('.vault-courses.horizontal')
			const before = horizontal ? state.root.scrollLeft : state.root.scrollTop
			if (files && state.root.contains(files) && state.axis === 'y') {
				const remainder = scrollElement(files, 'y', dy)
				if (!horizontal && Math.abs(remainder) > .1) scrollElement(state.root, 'y', remainder)
			} else scrollElement(state.root, horizontal ? 'x' : 'y', horizontal ? dx : dy)
			if (Math.abs((horizontal ? state.root.scrollLeft : state.root.scrollTop) - before) > .1) state.rootMoved = true
		}
		state.previous = point
	}
	function up(event: PointerEvent) {
		if (!gesture || event.pointerId !== gesture.id) return
		if (!gesture.target.isConnected || !gesture.root.isConnected) { cancel(); return }
		if ((gesture.moved || gesture.held) && event.cancelable) event.preventDefault()
		finish(false, { x: event.clientX, y: event.clientY })
	}
	function click(event: MouseEvent) {
		// detail=0 is a keyboard/assistive activation, not a synthesized pointer click.
		if (suppressAllClicks && Date.now() < suppressedUntil && event.detail !== 0) { event.preventDefault(); event.stopImmediatePropagation() }
	}
	function nativeDrag(event: DragEvent) { if (gesture) { event.preventDefault(); event.stopImmediatePropagation() } }
	function contextMenu(event: MouseEvent) { if (gesture) { event.preventDefault(); event.stopImmediatePropagation() } }
	function lost(event: PointerEvent) { if (event.target === observed && gesture?.id === event.pointerId) cancel() }
	function detach() {
		cancel()
		observed?.removeEventListener('contextmenu', contextMenu, true)
		observed?.removeEventListener('dragstart', nativeDrag, true)
		observed?.removeEventListener('pointerdown', down, true)
		observed?.removeEventListener('click', click, true)
		observed?.removeEventListener('dblclick', click, true)
		observed?.removeEventListener('lostpointercapture', lost)
		observed = null
	}
	function attach() {
		if (observed === surface.value) return
		detach(); observed = surface.value
		observed?.addEventListener('contextmenu', contextMenu, true)
		observed?.addEventListener('dragstart', nativeDrag, true)
		observed?.addEventListener('pointerdown', down, true)
		observed?.addEventListener('click', click, true)
		observed?.addEventListener('dblclick', click, true)
		observed?.addEventListener('lostpointercapture', lost)
	}
	watch(surface, attach, { flush: 'post' })
	onMounted(attach)
	onDeactivated(cancel)
	onUnmounted(detach)
	return { cancel }
}
