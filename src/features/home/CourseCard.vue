<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { Card, Checkbox, IconButton, Button, ContextMenu, type ButtonMenuOption } from '@modrinth/ui'
import { PlusIcon } from '@modrinth/assets'
import ArchiveTile from '@/components/ArchiveTile.vue'
import { subjectColor } from '@/platform/subject-colors'
import type { CourseView } from '@/platform/vault-types'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const props=defineProps<{course:CourseView;now:number}>()
const emit=defineEmits<{jump:[id:string]}>()
const vault=useVaultStore();const platform=usePlatform()
const menu=ref<InstanceType<typeof ContextMenu>|null>(null)
const selectable=computed(()=>props.course.items.filter(item=>item.state==='available'))
const selected=computed(()=>selectable.value.length>0&&selectable.value.every(item=>vault.selected.includes(item.id)))
const cardStyle=computed(()=>{
	const id=props.course.subjectId
	const subject=vault.config.timetable.subjects.find(subject=>subject.id===id)
	return id ? {'--course-color':subjectColor(id,subject?.color,vault.config.timetable.subjects)} : {}
})
const future=computed(()=>props.course.slotId!==null&&props.course.startUnixMs>props.now)
const current=computed(()=>props.course.slotId!==null&&props.course.startUnixMs<=props.now&&props.course.endUnixMs>props.now)
const siblings=computed(()=>vault.courses.filter(course=>course.subjectId&&course.subjectId===props.course.subjectId&&course.id!==props.course.id))
const before=computed(()=>siblings.value.filter(course=>course.startUnixMs<props.course.startUnixMs).at(-1))
const after=computed(()=>siblings.value.find(course=>course.startUnixMs>props.course.startUnixMs))
const short=(ms:number)=>new Date(ms).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'})
let press:ReturnType<typeof setTimeout>|undefined
async function change(subjectId:string){try{await platform.vault.changeCourse(props.course.id,subjectId);await vault.refresh(platform)}catch(error){vault.error=String(error)}}
function context(event:MouseEvent){const options:ButtonMenuOption[]=[{type:'heading',label:'只调整当天此课次'},...vault.config.timetable.subjects.map(subject=>({id:subject.id,label:subject.name,action:()=>{void change(subject.id)}})),{type:'divider'},{id:'cancel',label:'取消当天这节课',action:()=>{void change('')}}];menu.value?.open(event,options)}
function longPress(event:PointerEvent){if(event.pointerType==='mouse'||(event.target as HTMLElement).closest('button'))return;const synthetic=new MouseEvent('contextmenu',{clientX:event.clientX,clientY:event.clientY});press=setTimeout(()=>context(synthetic),600)}
function cancelPress(){if(press)clearTimeout(press)}
onUnmounted(cancelPress)
</script>
<template>
	<Card :id="`course-${course.id}`" class="vault-course-card" :class="{future,current,empty:!course.items.length}" :style="cardStyle" :data-course-id="course.id" :data-file-count="course.items.length" :aria-label="`${course.subjectName} · ${course.label}`">
		<header class="vault-course-heading" @contextmenu.prevent="context" @pointerdown="longPress" @pointerup="cancelPress" @pointercancel="cancelPress" @pointermove="cancelPress"><div class="min-w-0 grow"><h2 class="truncate">{{course.subjectName}}</h2><small v-if="course.slotId">{{course.label}} · {{short(course.startUnixMs)}}–{{short(course.endUnixMs)}}</small></div><Checkbox :label="`全选${course.subjectName}`" label-class="sr-only" :model-value="selected" :disabled="!selectable.length" @update:model-value="value=>vault.selectMany(selectable,value)"/><IconButton :label="`复制${course.subjectName}全部资料`" type="quiet" :disabled="!selectable.length||!vault.target?.writable||platform.mode!=='native'" @click="vault.requestExport(platform,selectable.map(item=>item.id))"><PlusIcon/></IconButton></header>
		<p v-if="future" class="vault-muted text-sm">这节课还没有上哦</p>
		<div v-if="course.items.length" class="vault-file-grid vault-course-files"><ArchiveTile v-for="item in course.items" :key="item.id" :item="item"/></div>
		<div v-else class="vault-course-empty">{{current?'上课中，等待收纳资料':'暂时没有文件'}}</div>
		<footer v-if="before||after" class="flex justify-between gap-2"><Button v-if="before" type="quiet" size="sm" @click="emit('jump',before.id)">前一节{{course.subjectName}}</Button><Button v-if="after" type="quiet" size="sm" class="ml-auto" @click="emit('jump',after.id)">后一节{{course.subjectName}}</Button></footer>
		<ContextMenu ref="menu" label="当天课程菜单"/>
	</Card>
</template>
