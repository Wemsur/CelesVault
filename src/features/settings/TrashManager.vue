<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Button, ConfirmModal } from '@modrinth/ui'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { ArchiveDetail } from '@/platform/vault-types'
const vault=useVaultStore();const platform=usePlatform();const items=ref<ArchiveDetail[]>([]);const error=ref('');const busy=ref(false)
const selected=ref<ArchiveDetail|null>(null);const purgeModal=ref<InstanceType<typeof ConfirmModal>|null>(null)
async function refresh(){try{items.value=await platform.vault.trashList()}catch(e){error.value=String(e)}}
async function restore(id:string){busy.value=true;error.value='';try{await platform.vault.restore(id);vault.archivesChanged();await refresh();await vault.refresh(platform)}catch(e){error.value=String(e)}finally{busy.value=false}}
async function purge(){if(!selected.value)return;busy.value=true;try{await platform.vault.purge(selected.value.item.id);await refresh()}catch(e){error.value=String(e)}finally{busy.value=false}}
onMounted(refresh)
</script>
<template>
	<section><div class="flex items-center justify-between mb-3"><h3>回收站</h3><Button type="outlined" @click="refresh">刷新</Button></div><p class="vault-muted text-sm mb-3">不会自动清理。移入这里的资料可恢复；永久清理需要单独确认，不删除原来源文件。</p>
		<p v-if="!items.length" class="vault-muted">回收站为空</p><div v-for="detail in items" :key="detail.item.id" class="vault-trash-row"><span class="truncate grow">{{detail.item.displayName}}</span><Button size="sm" :disabled="busy" @click="restore(detail.item.id)">恢复</Button><Button size="sm" type="outlined" :disabled="busy" @click="selected=detail;purgeModal?.show()">永久清理</Button></div><p v-if="error" class="vault-file-error" role="status">{{error}}</p>
		<ConfirmModal ref="purgeModal" title="永久清理这份归档？" :description="`将永久移除归档副本和相关记录：${selected?.item.displayName??''}。原来源不受影响，此操作不可恢复。`" proceed-label="永久清理" :has-to-type="true" confirmation-text="清理" @proceed="purge"/>
	</section>
</template>
