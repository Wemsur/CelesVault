<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { Checkbox, IconButton, Button, ConfirmModal, NewModal } from '@modrinth/ui'
import { FileTextIcon, FileImageIcon, InfoIcon, RefreshCwIcon, XIcon } from '@modrinth/assets'
import type { ArchiveItem } from '@/platform/vault-types'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import { allowsDemoInteractions } from '@/platform/demo-interactions'
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
	if (!taskId || actionBusy.value || !allowsDemoInteractions(platform)) return
	actionBusy.value = true
	try { await platform.vault.dismissFailed(taskId); vault.archivesChanged(); await vault.refresh(platform) }
	catch (error) { vault.error = String(error) }
	finally { actionBusy.value = false }
}
const imageTypes = new Set(['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'tiff', 'tif', 'heic', 'heif'])
let disposed = false
let suppressClick = false
function hold() {
	if (props.item.state !== 'available') return
	vault.beginDrag(props.item)
}
watch(() => [props.item.id,props.item.state] as const, async ([id]) => { image.value = null; if (props.item.state === 'available' && imageTypes.has(props.item.extension)) { try { const result = await archiveThumbnail(platform,id); if (!disposed && props.item.id === id) image.value = result } catch (failure) {vault.error=`缩略图未生成：${String(failure)}`} } }, { immediate: true })
function select() { if (props.item.state !== 'available') return; if (suppressClick) { suppressClick = false; return } if (!vault.selected.includes(props.item.id)) { vault.clearSelection(); vault.select(props.item, true) } }
function nativeDrag(event: DragEvent) {
	if (props.item.state !== 'available') { event.preventDefault(); return }
	suppressClick = true; vault.beginDrag(props.item)
	if (event.dataTransfer) { event.dataTransfer.effectAllowed = 'copy'; event.dataTransfer.setData('application/x-celesvault-archive', props.item.id) }
}
async function retry() {
	if (!props.item.taskId || actionBusy.value || !allowsDemoInteractions(platform)) return
	actionBusy.value = true
	try { await platform.vault.retry(props.item.taskId); vault.archivesChanged(); await vault.refresh(platform) }
	catch (error) { vault.error = String(error) }
	finally { actionBusy.value = false }
}
onUnmounted(() => { disposed = true })
</script>
<template>
	<div class="vault-file-tile" :class="{ selected: vault.selected.includes(item.id), compact, failed: item.state === 'failed' }" :data-archive-id="item.id" :data-archive-state="item.state" :draggable="item.state === 'available'" tabindex="0" role="group" :aria-label="item.displayName" @click="select" @dblclick="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.enter="item.state === 'available' && vault.openArchive(platform,item.id)" @keydown.space.prevent="vault.select(item,!vault.selected.includes(item.id))" @dragstart="nativeDrag" @dragend="vault.endDrag" @vault-hold="hold" @contextmenu.prevent>
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
					<Button size="sm" type="outlined" :loading="actionBusy" :disabled="!item.taskId || !allowsDemoInteractions(platform)" @click="confirmRemove"><XIcon />移除</Button>
					<Button size="sm" type="colored" color="red" :loading="actionBusy" :disabled="!item.taskId || !allowsDemoInteractions(platform)" @click="retry"><RefreshCwIcon />重试</Button>
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
