<script setup lang="ts">
import { ref, watch } from 'vue'
import { NewModal, ConfirmModal, Button, Input, DropdownSelect } from '@modrinth/ui'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const vault = useVaultStore(); const platform=usePlatform()
const modal=ref<InstanceType<typeof NewModal>|null>(null)
const deleteModal=ref<InstanceType<typeof ConfirmModal>|null>(null)
const name=ref(''); const destination=ref(''); const fromCourse=ref(''); const busy=ref(false); const error=ref('')
watch(()=>vault.detailOpen,open=>{if(open && vault.detail){name.value=vault.detail.item.displayName;destination.value='';fromCourse.value=vault.detail.uses[0]?.courseId??'';error.value='';modal.value?.show()}})
async function save(){if(!vault.detail)return;busy.value=true;error.value='';let saved=false;try{await platform.vault.editArchive(vault.detail.item.id,name.value,null);if(destination.value && fromCourse.value)await platform.vault.moveUsage(vault.detail.item.id,fromCourse.value,destination.value);await vault.refresh(platform);vault.detail=await platform.vault.detail(vault.detail.item.id);saved=true}catch(e){error.value=String(e)}finally{busy.value=false}if(saved)modal.value?.hide()}
async function remove(){if(!vault.detail)return;try{await platform.vault.trash(vault.detail.item.id);vault.archivesChanged();vault.clearSelection();await vault.refresh(platform);modal.value?.hide()}catch(e){error.value=String(e)}}
</script>
<template>
	<NewModal ref="modal" header="资料详情" width="40rem" scrollable :disable-close="busy" :on-hide="()=>{vault.detailOpen=false}">
		<template v-if="vault.detail"><label>显示名称<Input v-model="name" aria-label="资料显示名称" class="mt-2"/></label>
		<dl class="vault-diagnostics mt-4"><dt>类型 / 大小</dt><dd>{{ vault.detail.item.extension.toUpperCase() }} / {{ (vault.detail.item.size/1024).toFixed(1) }} KB</dd><dt>归档副本</dt><dd>{{ vault.detail.archivePath }}</dd><dt>原来源</dt><dd><div v-for="path in vault.detail.sourcePaths" :key="path">{{path}}</div></dd></dl>
		<h3 class="mt-4">使用历史</h3><ul class="vault-use-history"><li v-for="use in vault.detail.uses" :key="use.courseId"><span>{{use.date}} · {{use.subjectName}}</span><small>{{new Date(use.detectedUnixMs).toLocaleTimeString('zh-CN')}}</small></li></ul>
		<div v-if="vault.courses.length" class="grid gap-2 mt-4"><DropdownSelect v-model="fromCourse" name="纠正来源课次" aria-label="纠正来源课次" :options="vault.detail.uses.map(use=>use.courseId)" :display-name="id=>{const use=vault.detail?.uses.find(use=>use.courseId===id);return use ? `${use.date} ${use.subjectName}` : id}"/><DropdownSelect v-model="destination" name="纠正目标课次" aria-label="纠正目标课次" :options="['',...vault.courses.map(course=>course.id)]" :display-name="id=>id ? vault.courses.find(course=>course.id===id)?.subjectName??id : '不更改课程归属'"/></div>
		<p class="vault-muted text-xs mt-3">打开归档主副本；需要编辑时另存新文件。名称或课程纠正仅更改记录，不修改原资料。</p><p v-if="error" class="vault-file-error" role="status">{{error}}</p></template>
		<template #actions><Button type="outlined" @click="deleteModal?.show()">移入回收站</Button><Button type="outlined" @click="vault.detail && vault.openArchive(platform,vault.detail.item.id)">打开归档</Button><Button type="colored" color="brand" :loading="busy" @click="save">保存</Button></template>
	</NewModal>
	<ConfirmModal ref="deleteModal" title="将资料移入回收站？" description="所有使用记录会保留，资料暂不出现在库中。你可以在设置中恢复；原文件不会删除。" proceed-label="移入回收站" @proceed="remove"/>
</template>
