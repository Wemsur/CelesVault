<script setup lang="ts">
import { ref } from 'vue'
import { Button, IconButton, ConfirmModal, injectNotificationManager } from '@modrinth/ui'
import { CopyIcon, TrashIcon, XIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const vault = useVaultStore(); const platform = usePlatform()
const notifications = injectNotificationManager()
const trashModal = ref<InstanceType<typeof ConfirmModal> | null>(null)
const trashItems = ref<{ id: string; name: string }[]>([])
const trashBusy = ref(false)
function confirmTrash() {
	if (vault.busy || trashBusy.value || platform.mode !== 'native') return
	trashItems.value = vault.selectedAvailable.map(item => ({ id: item.id, name: item.displayName }))
	if (trashItems.value.length) trashModal.value?.show()
}
async function moveToTrash() {
	if (vault.busy || trashBusy.value || platform.mode !== 'native') return
	const batch = [...trashItems.value]
	if (!batch.length) return
	trashBusy.value = true; vault.busy = true; vault.error = ''
	const moved = new Set<string>(); const errors: string[] = []
	try {
		for (const item of batch) {
			try { await platform.vault.trash(item.id); moved.add(item.id) }
			catch (error) { errors.push(`${item.name}：${String(error)}`) }
		}
		vault.selected = vault.selected.filter(id => !moved.has(id))
		for (const id of moved) delete vault.selectedItems[id]
		if (moved.size) {
			vault.archivesChanged()
			const seconds = vault.config.app.notificationSeconds
			notifications.addNotification({ title: '已移入回收站', text: `${moved.size} 份资料可在设置的回收站中恢复。`, type: 'success', autoCloseMs: seconds === 0 ? null : seconds * 1000 })
		}
		if (errors.length) vault.error = `已移入 ${moved.size}/${batch.length} 份，未成功的资料仍保留选择。\n${errors.join('\n')}`
		await vault.refresh(platform)
	} finally { trashBusy.value = false; vault.busy = false }
}
</script>
<template>
	<div v-if="vault.selected.length" class="vault-copy-bar">
		<Button type="colored" color="brand" :disabled="!vault.target?.writable || !vault.selectedAvailable.length || vault.busy || platform.mode !== 'native'" @click="vault.requestExport(platform,vault.selected)"><CopyIcon />复制全部（{{ vault.selectedAvailable.length }}）</Button>
		<Button type="quiet" :disabled="trashBusy" @click="vault.clearSelection"><XIcon />清除</Button>
		<span v-if="!vault.target" class="text-xs vault-muted">请选择目标目录</span>
	</div>
	<IconButton v-if="vault.selectedAvailable.length" class="vault-trash-fab" label="移入回收站" title="移入回收站" type="colored" color="red" size="lg" :loading="trashBusy" :disabled="vault.busy || platform.mode !== 'native'" @click="confirmTrash"><TrashIcon /></IconButton>
	<ConfirmModal ref="trashModal" :title="`将选中的 ${trashItems.length} 份资料移入回收站？`" description="只移动应用保存的副本，不删除原来源文件。同一资料在其他课次中的条目也会隐藏，使用历史保留；可在设置的回收站恢复。" :markdown="false" proceed-label="移入回收站" @proceed="moveToTrash"><ul class="vault-trash-confirm-list"><li v-for="item in trashItems" :key="item.id">{{ item.name }}</li></ul></ConfirmModal>
	<div v-if="vault.pointerDrag" class="vault-drag-ghost" :style="{left:`${vault.pointerDrag.x + 16}px`,top:`${vault.pointerDrag.y + 16}px`}"><CopyIcon />复制 {{ vault.pointerDrag.count }} 份</div>
</template>
