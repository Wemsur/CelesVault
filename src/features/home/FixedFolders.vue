<script setup lang="ts">
import { computed, ref } from 'vue'
import { IconButton, Button, ContextMenu, NewModal, Input, type ButtonMenuOption } from '@modrinth/ui'
import { PinIcon, PlusIcon, FolderIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { FixedFolder } from '@/platform/vault-types'
const vault=useVaultStore();const platform=usePlatform()
const context=ref<InstanceType<typeof ContextMenu>|null>(null)
const aliasModal=ref<InstanceType<typeof NewModal>|null>(null);const editing=ref<FixedFolder|null>(null);const alias=ref('')
const selectedDrive=computed(()=>vault.drives.find(drive=>drive.volumeId===vault.target?.volumeId))
function deviceKey(drive:typeof vault.drives[number]){return drive.identityReliable&&drive.deviceId?drive.deviceId:!drive.isUsb?`local-volume:${drive.volumeId}`:null}
const folders=computed(()=>vault.config.favorites.filter(folder=>!selectedDrive.value||folder.deviceId===deviceKey(selectedDrive.value)).sort((a,b)=>a.position-b.position))
function path(folder:FixedFolder){const drive=vault.drives.find(drive=>deviceKey(drive)===folder.deviceId&&drive.volumeId===folder.volumeId);return drive?drive.root+folder.relativePath.replaceAll('/','\\'):null}
async function pin(){const drive=selectedDrive.value;const target=vault.target;if(!drive||!deviceKey(drive)||!target)return;const relativePath=target.path.slice(drive.root.length);if(vault.config.favorites.some(folder=>folder.deviceId===deviceKey(drive)&&folder.volumeId===drive.volumeId&&folder.relativePath===relativePath))return;const folder:FixedFolder={id:crypto.randomUUID(),deviceId:deviceKey(drive)!,volumeId:drive.volumeId,relativePath,alias:relativePath.split('\\').at(-1)||drive.name,position:vault.config.favorites.length};try{await platform.vault.favoriteSave(folder);await vault.loadConfig(platform)}catch(error){vault.error=String(error)}}
async function remove(folder:FixedFolder){try{await platform.vault.favoriteRemove(folder.id);await vault.loadConfig(platform)}catch(error){vault.error=String(error)}}
async function save(){if(!editing.value)return;try{await platform.vault.favoriteSave({...editing.value,alias:alias.value});await vault.loadConfig(platform);aliasModal.value?.hide()}catch(error){vault.error=String(error)}}
async function order(folder:FixedFolder,step:number){const list=[...folders.value];const at=list.findIndex(item=>item.id===folder.id);const other=list[at+step];if(!other)return;[list[at],list[at+step]]=[list[at+step],list[at]];try{const current=await platform.vault.config();const positions=new Map(list.map((item,index)=>[item.id,index]));current.favorites=current.favorites.map(item=>positions.has(item.id)?{...item,position:positions.get(item.id)!}:item);await vault.saveConfig(platform,current)}catch(error){vault.error=String(error)}}
async function relocate(folder:FixedFolder){const target=vault.target;const drive=selectedDrive.value;if(!target||!drive||!deviceKey(drive)||deviceKey(drive)!==folder.deviceId){vault.error='请先浏览同一设备上的新目录，再选择重新定位';return}try{await platform.vault.favoriteSave({...folder,volumeId:drive.volumeId,relativePath:target.path.slice(drive.root.length)});await vault.loadConfig(platform)}catch(error){vault.error=String(error)}}
function menu(event:MouseEvent,folder:FixedFolder){const options:ButtonMenuOption[]=[{id:'alias',label:'修改别名',action:()=>{editing.value=folder;alias.value=folder.alias;aliasModal.value?.show()}},{id:'up',label:'上移',action:()=>{void order(folder,-1)}},{id:'down',label:'下移',action:()=>{void order(folder,1)}},{id:'relocate',label:'重新定位到当前目录',action:()=>{void relocate(folder)}},{type:'divider'},{id:'remove',label:'移除固定',action:()=>{void remove(folder)}}];context.value?.open(event,options)}
</script>
<template>
	<section class="vault-fixed-folders"><header class="vault-panel-title"><PinIcon/><h2>固定文件夹</h2><IconButton class="ml-auto" label="固定当前文件夹" type="quiet" :disabled="!selectedDrive||!deviceKey(selectedDrive)||platform.mode!=='native'" @click="pin"><PlusIcon/></IconButton></header>
		<p v-if="!folders.length" class="vault-muted text-sm p-4">选择驱动器后，可固定常用目录。</p><p v-if="selectedDrive?.isUsb&&!selectedDrive.identityReliable" class="vault-file-error text-xs">设备身份不可靠，不能自动套用收藏。</p>
		<div class="vault-fixed-grid"><Button v-for="folder in folders" :key="folder.id" type="outlined" :disabled="!path(folder)" :data-copy-target="path(folder)??undefined" :title="`${folder.volumeId} · ${folder.relativePath||'根目录'}`" @click="path(folder)&&vault.browse(platform,path(folder)!)" @contextmenu.prevent="menu($event,folder)" @dragover.prevent @drop.prevent="path(folder)&&vault.dropAt(platform,path(folder)!)"><FolderIcon/><span class="truncate">{{folder.alias}}</span></Button></div>
		<ContextMenu ref="context" label="固定文件夹菜单"/><NewModal ref="aliasModal" header="固定文件夹别名" width="24rem"><Input v-model="alias" aria-label="固定夹别名"/><template #actions><Button type="outlined" @click="aliasModal?.hide()">取消</Button><Button type="colored" color="brand" @click="save">保存</Button></template></NewModal>
	</section>
</template>
