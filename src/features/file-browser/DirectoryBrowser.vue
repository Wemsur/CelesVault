<script setup lang="ts">
import { computed, onDeactivated, onMounted, onUnmounted, ref, watch } from 'vue'
import { Input, IconButton, Button, DropdownSelect, ContextMenu, NewModal, ConfirmModal, type ButtonMenuOption } from '@modrinth/ui'
import { FolderIcon, FileIcon, PinIcon, ChevronLeftIcon, ChevronRightIcon, RefreshCwIcon, PlusIcon, ListIcon, GridIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { DirectoryEntry, FileRecovery } from '@/platform/vault-types'
import { useSurfaceGesture } from '@/composables/surface-gesture'
const directorySurface=ref<HTMLElement|null>(null)
const props = withDefaults(defineProps<{ context?: 'home' | 'library' }>(), { context: 'home' })
const vault = useVaultStore(); const platform = usePlatform()
const target = computed(() => props.context === 'home' ? vault.homeTarget : vault.libraryBrowsing ? vault.libraryTarget : null)
const address = ref(''); const page = ref(0); const sort = ref('name'); const descending = ref(false)
const history = ref<string[]>([]); const historyIndex = ref(-1)
const menu = ref<InstanceType<typeof ContextMenu> | null>(null)
const editModal = ref<InstanceType<typeof NewModal> | null>(null)
const recycleModal = ref<InstanceType<typeof ConfirmModal> | null>(null)
const editName = ref(''); const edited = ref<DirectoryEntry | null>(null); const editKind = ref<'create' | 'rename'>('create')
const localError = ref(''); const recoveries = ref<FileRecovery[]>([])
let unlisten: (() => void) | undefined; let disposed = false
watch(() => target.value?.path, value => {
	address.value = value ?? ''; page.value = 0
	if (!value) { recoveries.value = []; return }
	if (history.value[historyIndex.value]?.toLowerCase() !== value.toLowerCase()) {
		history.value = history.value.slice(0, historyIndex.value + 1)
		history.value.push(value); historyIndex.value = history.value.length - 1
	}
})
const parent = computed(() => {
	const path = target.value?.path.replace(/\\+$/, '')
	if (!path || path.length <= 2) return null
	const at = path.lastIndexOf('\\'); return at <= 2 ? path.slice(0, 2) + '\\' : path.slice(0, at)
})
const breadcrumbs = computed(() => {
	if (!target.value) return []
	const parts = target.value.path.replace(/\\+$/, '').split('\\')
	return parts.map((label, index) => ({ label, path: index === 0 ? label + '\\' : parts.slice(0, index + 1).join('\\') }))
})
const sorted = computed(() => [...(target.value?.entries ?? [])].sort((a, b) => {
	if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
	const value = sort.value === 'size' ? a.size - b.size : sort.value === 'time' ? a.modifiedUnixMs - b.modifiedUnixMs : sort.value === 'type' ? a.extension.localeCompare(b.extension) : a.name.localeCompare(b.name, 'zh-CN', { numeric: true })
	return descending.value ? -value : value
}))
const entries = computed(() => sorted.value.slice(page.value * 200, (page.value + 1) * 200))
const size = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`
async function navigate(path: string) {
	await vault.browse(platform, path, props.context)
	if (!target.value || target.value.path.toLowerCase() !== path.toLowerCase()) return
	await loadRecoveries()
}
async function travel(step: number) { const at = historyIndex.value + step; const path = history.value[at]; if (!path) return; const previous = historyIndex.value; historyIndex.value = at; await navigate(path); if (!target.value) historyIndex.value = previous }
async function refresh() { if (target.value) await navigate(target.value.path) }
function changeView(view: 'list' | 'grid') { vault.persistApp(platform, { browserView: view }) }
async function hidden() { vault.persistApp(platform, { showHidden: !vault.config.app.showHidden }); await refresh() }
async function open(entry: DirectoryEntry) { if (entry.isDir) { await navigate(entry.path); return } try { await platform.files.open(entry.path, entry.identity) } catch (error) { localError.value = String(error) } }
function create() { edited.value = null; editKind.value = 'create'; editName.value = ''; editModal.value?.show() }
function rename(entry: DirectoryEntry) { edited.value = entry; editKind.value = 'rename'; editName.value = entry.name; editModal.value?.show() }
async function saveName() {
	localError.value = ''
	try {
		if (editKind.value === 'create' && target.value) await platform.files.createFolder(target.value.path, target.value.identity, editName.value)
		else if (edited.value) await platform.files.rename(edited.value.path, edited.value.identity, editName.value)
		editModal.value?.hide(); await refresh()
	} catch (error) { localError.value = String(error) }
}
async function recycle() { if (!edited.value) return; try { await platform.files.recycle(edited.value.path, edited.value.identity); await refresh() } catch (error) { localError.value = String(error) } }
async function pinEntry(entry: DirectoryEntry, volumeId: string) {
	try { await vault.pinFolder(platform,{path:entry.path,identity:entry.identity,volumeId}) }
	catch(error){localError.value=String(error)}
}
async function unpin(id:string){try{await vault.unpinFolder(platform,id)}catch(error){localError.value=String(error)}}
function context(event: MouseEvent, entry: DirectoryEntry) {
	if(folderPointer?.dragging||Date.now()<suppressedUntil)return
	clearFolderGesture()
	const volumeId=target.value?.volumeId
	const pinned=volumeId&&entry.isDir?vault.pinnedFolder(entry.path,volumeId):undefined
	const options: ButtonMenuOption[] = [
		{ id: 'open', label: entry.isDir ? '进入文件夹' : '用系统应用打开', action: () => { void open(entry) } },
		{ id: 'reveal', label: '在资源管理器中显示', action: () => { void platform.files.reveal(entry.path, entry.identity).catch(error => localError.value = String(error)) } },
		{ id: 'rename', label: '重命名', action: () => rename(entry) },
		{ type: 'divider' }, { id: 'recycle', label: '移入系统回收站', action: () => { edited.value = entry; recycleModal.value?.show() } },
	]
	if(entry.isDir&&volumeId)options.splice(1,0,pinned
		?{id:'unpin',label:'取消固定',action:()=>{void unpin(pinned.id)}}
		:{id:'pin',label:'固定文件夹',action:()=>{void pinEntry(entry,volumeId)}})
	menu.value?.open(event, options)
}
let folderPointer: { id:number; entry:DirectoryEntry; volumeId:string; x:number; y:number; dragging:boolean; node:HTMLElement } | null=null
let folderPress:ReturnType<typeof setTimeout>|undefined
let suppressedUntil=0
const folderGhost=ref<{x:number;y:number;name:string}|null>(null)
function heldEntry(node:HTMLElement){const path=node.closest<HTMLElement>('[data-entry-path]')?.dataset.entryPath;return target.value?.entries.find(entry=>entry.path===path&&entry.isDir)}
const touchGesture=useSurfaceGesture(directorySurface,{
	holdFromControl:true,
	canHold:node=>platform.mode==='native'&&!!heldEntry(node),
	hold:(node,point)=>{const entry=heldEntry(node);const directory=target.value;if(!entry||!directory)return;vault.folderDrag={path:entry.path,identity:entry.identity,volumeId:directory.volumeId};folderGhost.value={...point,name:entry.name}},
	drag:point=>{if(folderGhost.value)folderGhost.value={...point,name:folderGhost.value.name}},
	drop:point=>{const source=vault.folderDrag;vault.folderDrag=null;folderGhost.value=null;if(source&&document.elementFromPoint(point.x,point.y)?.closest('[data-pin-folder-target]'))void vault.pinFolder(platform,source).catch(error=>{localError.value=String(error)})},
	cancel:()=>{folderGhost.value=null;vault.folderDrag=null},
})
function clearFolderGesture(){
	touchGesture.cancel()
	if(folderPress)clearTimeout(folderPress);folderPress=undefined
	const gesture=folderPointer;folderPointer=null;folderGhost.value=null;vault.folderDrag=null
	if(gesture?.node.hasPointerCapture(gesture.id))gesture.node.releasePointerCapture(gesture.id)
}
function folderDown(event:PointerEvent,entry:DirectoryEntry){
	if(event.pointerType!=='mouse')return
	if(!event.isPrimary){clearFolderGesture();return}
	if(!entry.isDir||event.button!==0||platform.mode!=='native'||!target.value)return
	clearFolderGesture()
	const node=event.currentTarget as HTMLElement
	folderPointer={id:event.pointerId,entry:{...entry},volumeId:target.value.volumeId,x:event.clientX,y:event.clientY,dragging:false,node}
	node.setPointerCapture(event.pointerId)
	folderPress=setTimeout(()=>{const gesture=folderPointer;if(!gesture)return;gesture.dragging=true;vault.folderDrag={path:gesture.entry.path,identity:gesture.entry.identity,volumeId:gesture.volumeId};folderGhost.value={x:gesture.x,y:gesture.y,name:gesture.entry.name};suppressedUntil=Date.now()+800},450)
}
function folderMove(event:PointerEvent){
	const gesture=folderPointer;if(!gesture||event.pointerId!==gesture.id)return
	if(!gesture.dragging&&Math.hypot(event.clientX-gesture.x,event.clientY-gesture.y)>10){
		if(folderPress)clearTimeout(folderPress)
		gesture.dragging=true;vault.folderDrag={path:gesture.entry.path,identity:gesture.entry.identity,volumeId:gesture.volumeId}
	}
	if(gesture.dragging){if(event.cancelable)event.preventDefault();folderGhost.value={x:event.clientX,y:event.clientY,name:gesture.entry.name};suppressedUntil=Date.now()+800}
}
async function folderUp(event:PointerEvent){
	const gesture=folderPointer;if(!gesture||gesture.id!==event.pointerId)return
	const drop=gesture.dragging&&document.elementFromPoint(event.clientX,event.clientY)?.closest('[data-pin-folder-target]')
	const source=drop?vault.folderDrag:null
	if(gesture.dragging)suppressedUntil=Date.now()+800
	clearFolderGesture()
	if(source){try{await vault.pinFolder(platform,source)}catch(error){localError.value=String(error)}}
}
function folderClick(event:MouseEvent){if(Date.now()<suppressedUntil){event.preventDefault();event.stopPropagation()}}
watch(()=>target.value?.identity,clearFolderGesture)
async function loadRecoveries() { const current = target.value; if (!current || platform.mode !== 'native') { recoveries.value = []; return } try { const values = await platform.files.recoveries(current.path, current.identity); if (target.value?.identity === current.identity) recoveries.value = values } catch (error) { localError.value = String(error) } }
async function restore(recovery: FileRecovery) { const current = target.value; if (!current) return; try { await platform.files.restoreRecovery(current.path, current.identity, recovery.id); await refresh() } catch (error) { localError.value = String(error) } }
onMounted(async () => {
	window.addEventListener('blur',clearFolderGesture)
	try { unlisten = await platform.files.onDirectoryChanged(() => { if (vault.targetContext === props.context) void refresh() }); if (disposed) unlisten() } catch (error) { localError.value = String(error) }
})
onDeactivated(clearFolderGesture)
onUnmounted(() => { disposed = true; clearFolderGesture(); window.removeEventListener('blur',clearFolderGesture); unlisten?.() })
</script>
<template>
	<div class="vault-browser" :data-copy-target="target?.path" @dragover.prevent @drop.prevent="target && vault.dropAt(platform, target.path)">
		<div class="vault-browser-toolbar">
			<IconButton label="后退" type="quiet" :disabled="historyIndex <= 0 || vault.busy" @click="travel(-1)"><ChevronLeftIcon /></IconButton>
			<IconButton label="前进" type="quiet" :disabled="historyIndex + 1 >= history.length || vault.busy" @click="travel(1)"><ChevronRightIcon /></IconButton>
			<IconButton label="上级目录" type="quiet" :disabled="!parent || vault.busy" @click="parent && navigate(parent)"><FolderIcon /></IconButton>
			<Input v-model="address" placeholder="选择驱动器或输入本地路径" aria-label="目标目录路径" class="grow min-w-0" @keydown.enter="navigate(address)" />
			<IconButton label="刷新目录" type="quiet" :disabled="!target || vault.busy" @click="refresh"><RefreshCwIcon /></IconButton>
		</div>
		<div v-if="target" class="vault-breadcrumbs"><Button v-for="crumb in breadcrumbs" :key="crumb.path" type="quiet" size="sm" @click="navigate(crumb.path)">{{ crumb.label }}</Button></div>
		<div v-if="target" class="vault-browser-options"><IconButton label="新建文件夹" type="quiet" :disabled="!target.writable" @click="create"><PlusIcon /></IconButton><DropdownSelect v-model="sort" name="目录排序" aria-label="目录排序" :options="['name','type','size','time']" :display-name="value => ({name:'名称',type:'类型',size:'大小',time:'时间'}[value as string] ?? value)" /><Button type="quiet" size="sm" @click="descending = !descending">{{ descending ? '降序' : '升序' }}</Button><IconButton label="网格视图" type="quiet" @click="changeView('grid')"><GridIcon /></IconButton><IconButton label="列表视图" type="quiet" @click="changeView('list')"><ListIcon /></IconButton><Button type="quiet" size="sm" :aria-pressed="vault.config.app.showHidden" @click="hidden">隐藏项</Button></div>
		<div v-if="!target" class="vault-empty"><FolderIcon /><strong>未选择目标目录</strong><p>点击左侧驱动器。复制不会写入另一个隐含目录。</p></div>
		<div v-else ref="directorySurface" class="vault-directory-list" :class="{grid: vault.config.app.browserView === 'grid'}">
			<button v-for="entry in entries" :key="entry.path" class="vault-directory-entry" :data-entry-path="entry.path" :class="{'vault-draggable-folder':entry.isDir}" :data-copy-target="entry.isDir ? entry.path : undefined" @pointerdown="folderDown($event,entry)" @pointermove="folderMove" @pointerup="folderUp" @pointercancel="event=>{if(event.pointerType==='mouse')clearFolderGesture()}" @lostpointercapture="event=>{if(event.pointerType==='mouse')clearFolderGesture()}" @dragstart.prevent @click.capture="folderClick" @dblclick="Date.now()>=suppressedUntil&&open(entry)" @keydown.enter="open(entry)" @contextmenu.prevent="context($event,entry)" @dragover="entry.isDir && $event.preventDefault()" @drop.stop.prevent="entry.isDir && vault.dropAt(platform,entry.path)"><component :is="entry.isDir ? FolderIcon : FileIcon" /><span :title="entry.name">{{entry.name}}</span><small>{{entry.isDir ? '文件夹' : size(entry.size)}}</small></button>
			<p v-if="!target.entries.length" class="vault-muted p-4">此文件夹为空</p>
		</div>
		<p v-if="localError || vault.error" class="vault-file-error text-xs p-2" role="status">{{localError || vault.error}}</p>
		<div v-if="recoveries.length" class="vault-recovery-list"><strong>未完成替换的备份</strong><div v-for="item in recoveries" :key="item.id"><span>{{item.originalName}}</span><Button size="sm" :disabled="!item.canRestore" @click="restore(item)">恢复旧文件</Button><small v-if="item.error" class="vault-file-error">{{item.error}}</small></div></div>
		<div v-if="target && sorted.length > 200" class="flex items-center gap-2 p-2"><Button :disabled="page === 0" @click="page--">上一页</Button><span>{{page + 1}}</span><Button :disabled="(page + 1) * 200 >= sorted.length" @click="page++">下一页</Button></div>
		<Teleport to="body"><div v-if="folderGhost" class="vault-drag-ghost vault-folder-drag-ghost" :style="{left:`${folderGhost.x+16}px`,top:`${folderGhost.y+16}px`}"><PinIcon/><span class="truncate">固定 {{folderGhost.name}}</span></div></Teleport>
		<ContextMenu ref="menu" label="目录项操作" /><NewModal ref="editModal" :header="editKind === 'create' ? '新建文件夹' : '重命名'" width="26rem"><Input v-model="editName" aria-label="文件或文件夹名称" /><p v-if="localError" class="vault-file-error text-sm">{{localError}}</p><template #actions><Button type="outlined" @click="editModal?.hide()">取消</Button><Button type="colored" color="brand" @click="saveName">保存</Button></template></NewModal>
		<ConfirmModal ref="recycleModal" title="将选中项移入系统回收站？" description="不能确认回收站支持时会拒绝操作，不会降级为永久删除；USB删除默认禁用。" proceed-label="移入回收站" @proceed="recycle" />
	</div>
</template>
