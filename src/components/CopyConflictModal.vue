<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { NewModal, Button, DropdownSelect, Checkbox } from '@modrinth/ui'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { ConflictPolicy } from '@/platform/vault-types'
const vault = useVaultStore(); const platform = usePlatform()
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const applyAll = ref<ConflictPolicy>('keepBoth')
const replaceApproved = ref(false)
const labels: Record<ConflictPolicy,string> = { keepBoth:'保留两份',skip:'跳过',replace:'替换' }
const replacing = computed(() => Object.values(vault.policies).includes('replace'))
watch(() => vault.plan, plan => { if (plan) { replaceApproved.value=false; modal.value?.show() } })
function batch() { if (vault.plan) for (const item of vault.plan.items) if (item.exists) vault.policies[item.sourceId]=applyAll.value }
async function commit() { await vault.commitPlan(platform); if (!vault.plan) modal.value?.hide() }
</script>
<template>
	<NewModal ref="modal" header="目标中已有同名文件" width="44rem" scrollable :disable-close="vault.busy" :on-hide="() => { if(!vault.busy) vault.plan=null }">
		<p class="vault-muted">默认保留两份，不自动覆盖。目标或旧文件在确认后变化时会停止并保留原文件。</p>
		<div class="flex gap-2 items-center my-4"><DropdownSelect v-model="applyAll" name="批次冲突策略" aria-label="批次冲突策略" :options="['keepBoth','skip','replace']" :display-name="value=>labels[value as ConflictPolicy]"/><Button type="outlined" @click="batch">应用到本批次冲突</Button></div>
		<div v-for="item in vault.plan?.items.filter(item=>item.exists) ?? []" :key="item.sourceId" class="vault-conflict-row"><span class="min-w-0 grow truncate" :title="item.sourceName">{{ item.sourceName }}</span><small class="vault-muted">{{ item.existingSize === null ? '文件夹' : `${(item.existingSize/1024).toFixed(1)} KB` }}</small><DropdownSelect v-model="vault.policies[item.sourceId]" :name="`冲突-${item.sourceId}`" :aria-label="`${item.sourceName}的冲突策略`" :options="['keepBoth','skip','replace']" :display-name="value=>labels[value as ConflictPolicy]"/></div>
		<Checkbox v-if="replacing" v-model="replaceApproved" label="我确认替换选定的旧文件；未选替换的文件不受影响" class="mt-4"/>
		<p v-if="vault.error" class="vault-file-error mt-3" role="status">{{ vault.error }}</p>
		<template #actions><Button type="outlined" :disabled="vault.busy" @click="modal?.hide()">取消</Button><Button type="colored" color="brand" :disabled="vault.busy || (replacing && !replaceApproved)" :loading="vault.busy" @click="commit">{{ replacing ? '确认替换并复制' : '开始复制' }}</Button></template>
	</NewModal>
</template>
