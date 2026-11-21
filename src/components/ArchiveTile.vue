<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { Checkbox, IconButton, Button } from '@modrinth/ui'
import { FileTextIcon, FileImageIcon, InfoIcon } from '@modrinth/assets'
import type { ArchiveItem } from '@/platform/vault-types'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import { archiveThumbnail } from '@/platform/thumbnail-cache'
const props = defineProps<{ item: ArchiveItem; compact?: boolean }>()
const vault = useVaultStore()
const platform = usePlatform()
const image = ref<string | null>(null)
const imageTypes = new Set(['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'tiff', 'tif', 'heic', 'heif'])
let disposed = false
let longPress: ReturnType<typeof setTimeout> | undefined
let dragging = false
let origin = { x: 0, y: 0 }
let suppressClick = false
let touchDrag = false
let touchOrigin = { x: 0, y: 0 }
function touchStart(event: TouchEvent) {
	if (event.touches.length !== 1 || props.item.state !== 'available' || (event.target as HTMLElement).closest('button')) return
	const touch = event.touches[0]
	touchOrigin = { x: touch.clientX, y: touch.clientY }
	if (longPress) clearTimeout(longPress)
	longPress = setTimeout(() => { vault.beginDrag(props.item); touchDrag = true; suppressClick = true; vault.pointerDrag = { ...touchOrigin, count: vault.dragIds.length } }, 450)
}
function touchMove(event: TouchEvent) {
	const touch = event.touches[0]
	if (!touch) return
	if (!touchDrag) { if (Math.hypot(touch.clientX - touchOrigin.x, touch.clientY - touchOrigin.y) > 10 && longPress) clearTimeout(longPress); return }
	if (event.cancelable) event.preventDefault()
	vault.pointerDrag = { x: touch.clientX, y: touch.clientY, count: vault.dragIds.length }
}
function touchEnd(event: TouchEvent) {
	if (longPress) clearTimeout(longPress)
	if (!touchDrag) return
	touchDrag = false
	if (event.cancelable) event.preventDefault()
	const touch = event.changedTouches[0]
	const target = touch && document.elementFromPoint(touch.clientX, touch.clientY)?.closest<HTMLElement>('[data-copy-target]')?.dataset.copyTarget
	if (target) void vault.dropAt(platform, target); else vault.endDrag()
}
function cancelGesture() { if (longPress) clearTimeout(longPress); dragging = false; touchDrag = false; vault.endDrag() }
watch(() => [props.item.id,props.item.state] as const, async ([id]) => { image.value = null; if (props.item.state === 'available' && imageTypes.has(props.item.extension)) { try { const result = await archiveThumbnail(platform,id); if (!disposed && props.item.id === id) image.value = result } catch (failure) {vault.error=`缩略图未生成：${String(failure)}`} } }, { immediate: true })
function select() { if (suppressClick) { suppressClick = false; return } if (!vault.selected.includes(props.item.id)) { vault.clearSelection(); vault.select(props.item, true) } }
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
	if (!dragging) { if (Math.hypot(event.clientX - origin.x, event.clientY - origin.y) > 10 && longPress) clearTimeout(longPress); return }
	event.preventDefault(); vault.pointerDrag = { x: event.clientX, y: event.clientY, count: vault.dragIds.length }
}
function up(event: PointerEvent) {
	if (longPress) clearTimeout(longPress)
	if (!dragging) return
	dragging = false
	const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-copy-target]')?.dataset.copyTarget
	if (target) void vault.dropAt(platform, target); else vault.endDrag()
}
function nativeDrag(event: DragEvent) { if (props.item.state !== 'available') { event.preventDefault(); return } suppressClick = true; vault.beginDrag(props.item); if (event.dataTransfer) { event.dataTransfer.effectAllowed = 'copy'; event.dataTransfer.setData('application/x-celesvault-archive', props.item.id) } }
async function retry() { if (!props.item.taskId) return; try { await platform.vault.retry(props.item.taskId); await vault.refresh(platform) } catch (error) { vault.error = String(error) } }
onUnmounted(() => { disposed = true; if (longPress) clearTimeout(longPress) })
</script>
<template>
	<div class="vault-file-tile" :class="{ selected: vault.selected.includes(item.id), compact }" :draggable="item.state === 'available'" tabindex="0" role="group" :aria-label="item.displayName" @click="select" @dblclick="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.enter="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.space.prevent="vault.select(item,!vault.selected.includes(item.id))" @dragstart="nativeDrag" @dragend="vault.endDrag" @pointerdown="down" @pointermove="move" @pointerup="up" @pointercancel="() => { if(dragging) cancelGesture() }" @touchstart="touchStart" @touchmove="touchMove" @touchend="touchEnd" @touchcancel="cancelGesture" @contextmenu.prevent>
		<Checkbox :label="`选择 ${item.displayName}`" label-class="sr-only" class="vault-file-check" :model-value="vault.selected.includes(item.id)" :disabled="item.state !== 'available'" @click.stop @update:model-value="value => vault.select(item,value)" />
		<img v-if="image" :src="image" class="vault-file-icon" alt="" loading="lazy" />
		<component v-else :is="imageTypes.has(item.extension) ? FileImageIcon : FileTextIcon" class="vault-file-icon" />
		<span class="vault-file-name" :title="item.displayName">{{ item.displayName }}</span>
		<small>{{ new Date(item.detectedUnixMs).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'}) }}</small>
		<small v-if="item.state !== 'available'" class="vault-file-error">{{ item.state === 'failed' ? '保存失败' : '等待保存' }}</small>
		<Button v-if="item.state === 'failed'" size="sm" @click.stop="retry">重试</Button>
		<IconButton v-if="item.state === 'available'" label="资料详情" type="quiet" size="sm" class="vault-file-info" @click.stop="vault.showDetail(platform,item.id)"><InfoIcon /></IconButton>
		<small v-if="item.error" class="vault-file-error">{{ item.error }}</small>
	</div>
</template>
