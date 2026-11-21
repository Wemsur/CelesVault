<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { Checkbox, IconButton, Button, ConfirmModal, NewModal } from '@modrinth/ui'
import { FileTextIcon, FileImageIcon, InfoIcon, RefreshCwIcon, XIcon } from '@modrinth/assets'
import type { ArchiveItem } from '@/platform/vault-types'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import { archiveThumbnail } from '@/platform/thumbnail-cache'
const props = defineProps<{ item: ArchiveItem; compact?: boolean }>()
const vault = useVaultStore()
const platform = usePlatform()
const image = ref<string | null>(null)
const actionBusy = ref(false)
const removeModal = ref<InstanceType<typeof ConfirmModal> | null>(null)
const removalTask = ref<string | null>(null)
const removalName = ref('')
const errorModal = ref<InstanceType<typeof NewModal> | null>(null)
const errorDetails = ref({ name: '', message: '' })
function showErrorDetails() {
	errorDetails.value = { name: props.item.displayName, message: failureText.value }
	errorModal.value?.show()
}
const failureText = computed(() => props.item.error || '这份资料未能完成收纳，可重试或移除此失败记录。')
function confirmRemove() {
	if (actionBusy.value || props.item.state !== 'failed' || !props.item.taskId) return
	removalTask.value = props.item.taskId; removalName.value = props.item.displayName; removeModal.value?.show()
}
async function removeFailed() {
	const taskId = removalTask.value
	if (!taskId || actionBusy.value || platform.mode !== 'native') return
	actionBusy.value = true
	try { await platform.vault.dismissFailed(taskId); vault.archivesChanged(); await vault.refresh(platform) }
	catch (error) { vault.error = String(error) }
	finally { actionBusy.value = false }
}
const imageTypes = new Set(['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'tiff', 'tif', 'heic', 'heif'])
let disposed = false
let longPress: ReturnType<typeof setTimeout> | undefined
let dragging = false
let origin = { x: 0, y: 0 }
let suppressClick = false
let touchDrag = false
let touchOrigin = { x: 0, y: 0 }
let touchPrevious = { x: 0, y: 0 }
let touchScrolling = false
let touchActive = false
let touchScroller: HTMLElement | null = null
function finishTouchScroll(center = false) {
	const root = touchScroller
	if (!root) return
	touchScroller = null
	const horizontal = root.classList.contains('horizontal')
	let destination = horizontal ? root.scrollLeft : root.scrollTop
	const maximum = horizontal ? root.scrollWidth - root.clientWidth : root.scrollHeight - root.clientHeight
	if (center && destination <= 2) destination = 0
	else if (center && destination >= maximum - 2) destination = maximum
	else if (center && root.isConnected) {
		const bounds = root.getBoundingClientRect()
		const centerPoint = horizontal ? bounds.left + root.clientWidth / 2 : bounds.top + root.clientHeight / 2
		let nearestDistance = Infinity
		for (const card of root.querySelectorAll<HTMLElement>('[data-course-id]')) {
			const box = card.getBoundingClientRect()
			const delta = (horizontal ? box.left + box.width / 2 : box.top + box.height / 2) - centerPoint
			if (Math.abs(delta) < nearestDistance) { nearestDistance = Math.abs(delta); destination = (horizontal ? root.scrollLeft : root.scrollTop) + delta }
		}
	}
	root.style.removeProperty('scroll-snap-type')
	if (center && root.isConnected) {
		const behavior = matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth'
		root.scrollTo(horizontal ? { left: destination, behavior } : { top: destination, behavior })
	}
}
function touchStart(event: TouchEvent) {
	if(event.touches.length!==1){cancelGesture();return}
	if ((event.target as HTMLElement).closest('button,input,label')) return
	const touch = event.touches[0]
	touchOrigin = { x: touch.clientX, y: touch.clientY }
	touchPrevious = { ...touchOrigin }; touchScrolling = false; touchActive = true; suppressClick=false
	if (longPress) clearTimeout(longPress)
	if(props.item.state==='available')longPress = setTimeout(() => { vault.beginDrag(props.item); touchDrag = true; suppressClick = true; vault.pointerDrag = { ...touchOrigin, count: vault.dragIds.length } }, 450)
}
function touchMove(event: TouchEvent) {
	const touch = event.touches[0]
	if (!touch || !touchActive) return
	// tile禁止浏览器抢走长按手势；短滑动由这里交给最近的可滚动文件区/课次条。
	if (event.cancelable) event.preventDefault()
	if (!touchDrag) {
		if (!touchScrolling && Math.hypot(touch.clientX-touchOrigin.x,touch.clientY-touchOrigin.y)>10) {
			touchScrolling=true;suppressClick=true;if(longPress)clearTimeout(longPress)
		}
		if(touchScrolling){
			const node=event.currentTarget as HTMLElement
			const dx=touchPrevious.x-touch.clientX;const dy=touchPrevious.y-touch.clientY
			const files=node.closest<HTMLElement>('.vault-course-files')
			const courses=node.closest<HTMLElement>('.vault-courses')
			const fileCanScroll=files&&((dy>0&&files.scrollTop+files.clientHeight<files.scrollHeight-1)||(dy<0&&files.scrollTop>0))
			if(files&&fileCanScroll&&Math.abs(dy)>=Math.abs(dx)) files.scrollTop+=dy
			else if(courses) {
				if(!touchScroller){touchScroller=courses;courses.style.scrollSnapType='none'}
				if(courses.classList.contains('horizontal'))courses.scrollLeft+=dx
				else courses.scrollTop+=dy
			} else {
				let container=node.parentElement
				while(container&&container.scrollHeight<=container.clientHeight)container=container.parentElement
				if(container)container.scrollTop+=dy
			}
			touchPrevious={x:touch.clientX,y:touch.clientY}
		}
		return
	}
	vault.pointerDrag = { x: touch.clientX, y: touch.clientY, count: vault.dragIds.length }
}
function touchEnd(event: TouchEvent) {
	touchActive=false;finishTouchScroll(true)
	if (longPress) clearTimeout(longPress)
	if (!touchDrag) return
	touchDrag = false
	if (event.cancelable) event.preventDefault()
	const touch = event.changedTouches[0]
	const target = touch && document.elementFromPoint(touch.clientX, touch.clientY)?.closest<HTMLElement>('[data-copy-target]')?.dataset.copyTarget
	if (target) void vault.dropAt(platform, target); else vault.endDrag()
}
function cancelGesture() { finishTouchScroll(); if (longPress) clearTimeout(longPress); dragging = false; touchDrag = false; touchActive=false; vault.endDrag() }
watch(() => [props.item.id,props.item.state] as const, async ([id]) => { image.value = null; if (props.item.state === 'available' && imageTypes.has(props.item.extension)) { try { const result = await archiveThumbnail(platform,id); if (!disposed && props.item.id === id) image.value = result } catch (failure) {vault.error=`缩略图未生成：${String(failure)}`} } }, { immediate: true })
function select() { if (props.item.state !== 'available') return; if (suppressClick) { suppressClick = false; return } if (!vault.selected.includes(props.item.id)) { vault.clearSelection(); vault.select(props.item, true) } }
function down(event: PointerEvent) {
	if (event.pointerType !== 'pen' || props.item.state !== 'available' || (event.target as HTMLElement).closest('button')) return
	origin = { x: event.clientX, y: event.clientY }
	const element = event.currentTarget as HTMLElement
	longPress = setTimeout(() => {
		vault.beginDrag(props.item); dragging = true; suppressClick = true
		element.setPointerCapture(event.pointerId)
		vault.pointerDrag = { ...origin, count: vault.dragIds.length }
	}, 450)
}
function move(event: PointerEvent) {
	if(event.pointerType!=='pen')return
	if (!dragging) { if (Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 10 && longPress) clearTimeout(longPress); return }
	event.preventDefault(); vault.pointerDrag = { x: event.clientX, y: event.clientY, count: vault.dragIds.length }
}
function up(event: PointerEvent) {
	if(event.pointerType!=='pen')return
	if (longPress) clearTimeout(longPress)
	if (!dragging) return
	dragging = false
	const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-copy-target]')?.dataset.copyTarget
	if (target) void vault.dropAt(platform, target); else vault.endDrag()
}
function nativeDrag(event: DragEvent) { if (touchActive || props.item.state !== 'available') { event.preventDefault(); return } suppressClick = true; vault.beginDrag(props.item); if (event.dataTransfer) { event.dataTransfer.effectAllowed = 'copy'; event.dataTransfer.setData('application/x-celesvault-archive', props.item.id) } }
async function retry() {
	if (!props.item.taskId || actionBusy.value || platform.mode !== 'native') return
	actionBusy.value = true
	try { await platform.vault.retry(props.item.taskId); vault.archivesChanged(); await vault.refresh(platform) }
	catch (error) { vault.error = String(error) }
	finally { actionBusy.value = false }
}
onUnmounted(() => { disposed = true; if(touchDrag||dragging)cancelGesture(); else finishTouchScroll(); if (longPress) clearTimeout(longPress) })
</script>
<template>
	<div class="vault-file-tile" :class="{ selected: vault.selected.includes(item.id), compact, failed: item.state === 'failed' }" :draggable="item.state === 'available'" tabindex="0" role="group" :aria-label="item.displayName" @click="select" @dblclick="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.enter="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.space.prevent="vault.select(item,!vault.selected.includes(item.id))" @dragstart="nativeDrag" @dragend="vault.endDrag" @pointerdown="down" @pointermove="move" @pointerup="up" @pointercancel="event => { if(event.pointerType==='pen') cancelGesture() }" @touchstart="touchStart" @touchmove="touchMove" @touchend="touchEnd" @touchcancel="cancelGesture" @contextmenu.prevent>
		<Checkbox :label="`选择 ${item.displayName}`" label-class="sr-only" class="vault-file-check" :model-value="vault.selected.includes(item.id)" :disabled="item.state !== 'available'" @click.stop @update:model-value="value => vault.select(item,value)" />
		<img v-if="image" :src="image" class="vault-file-icon" alt="" loading="lazy" />
		<component v-else :is="imageTypes.has(item.extension) ? FileImageIcon : FileTextIcon" class="vault-file-icon" />
		<span class="vault-file-name" :title="item.displayName">{{ item.displayName }}</span>
		<small>{{ new Date(item.detectedUnixMs).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'}) }}</small>
		<small v-if="item.state === 'pending'" class="vault-muted">等待保存</small>
		<IconButton v-if="item.state === 'available'" label="资料详情" type="quiet" size="sm" class="vault-file-info" @click.stop="vault.showDetail(platform,item.id)"><InfoIcon /></IconButton>
		<div v-if="item.state === 'failed'" class="vault-failure-overlay">
			<div class="vault-failure-content">
				<strong>收纳失败</strong>
				<span class="vault-failure-name" :title="item.displayName">{{ item.displayName }}</span>
				<button type="button" class="vault-failure-details" @pointerdown.stop @touchstart.stop @touchmove.stop @touchend.stop @click.stop="showErrorDetails" @dblclick.stop @keydown.stop>错误详情</button>
				<div class="vault-failure-actions" @pointerdown.stop @touchstart.stop @touchmove.stop @touchend.stop @click.stop @dblclick.stop @keydown.stop>
					<Button size="sm" type="outlined" :loading="actionBusy" :disabled="!item.taskId || platform.mode !== 'native'" @click="confirmRemove"><XIcon />移除</Button>
					<Button size="sm" type="colored" color="red" :loading="actionBusy" :disabled="!item.taskId || platform.mode !== 'native'" @click="retry"><RefreshCwIcon />重试</Button>
				</div>
			</div>
		</div>
	</div>
	<NewModal ref="errorModal" header="收纳失败详情" width="32rem" scrollable>
		<p class="vault-error-detail-name">{{ errorDetails.name }}</p>
		<pre class="vault-error-detail-message">{{ errorDetails.message }}</pre>
		<template #actions><Button type="outlined" @click="errorModal?.hide()">关闭</Button></template>
	</NewModal>
	<ConfirmModal ref="removeModal" :title="`移除“${removalName}”的失败记录？`" description="只将失败记录从主页和资料库列表撤下，不删除原来源文件或任何归档、暂存文件。此操作不进入回收站，移除后不能再从此卡片重试。" :markdown="false" proceed-label="移除记录" @proceed="removeFailed"/>
</template>
