<script setup lang="ts">
import { computed, ref } from 'vue'
import { Button, Input, Checkbox, DropdownSelect, IconButton } from '@modrinth/ui'
import { PlusIcon, XIcon } from '@modrinth/assets'
import type { Timetable, WeekLesson } from '@/platform/vault-types'
const timetable = defineModel<Timetable>({ required: true })
const weekday = ref(1)
const days: Record<number,string> = {1:'星期一',2:'星期二',3:'星期三',4:'星期四',5:'星期五',6:'星期六',7:'星期日'}
const classSlots = computed(() => timetable.value.slots.filter(slot => slot.isClass))
function addSubject() { timetable.value.subjects.push({ id: crypto.randomUUID(), name: '' }) }
function removeSubject(id:string) { if(timetable.value.lessons.some(lesson=>lesson.subjectId===id)){alertText.value='请先修改引用该科目的星期课程，再移除科目';return} timetable.value.subjects=timetable.value.subjects.filter(subject=>subject.id!==id) }
function setClass(slotId:string, value:boolean) { const slot=timetable.value.slots.find(slot=>slot.id===slotId);if(!slot)return;slot.isClass=value;if(!value)timetable.value.lessons=timetable.value.lessons.filter(lesson=>lesson.slotId!==slotId) }
function addSlot() { timetable.value.slots.push({ id:crypto.randomUUID(),label:`第${classSlots.value.length+1}节`,start:'08:00',end:'08:40',isClass:true }) }
function removeSlot(id:string) { timetable.value.slots=timetable.value.slots.filter(slot=>slot.id!==id);timetable.value.lessons=timetable.value.lessons.filter(lesson=>lesson.slotId!==id) }
function lesson(slotId:string):WeekLesson|undefined { return timetable.value.lessons.find(lesson=>lesson.weekday===weekday.value&&lesson.slotId===slotId) }
function setLesson(slotId:string,patch:Partial<WeekLesson>) { const current=lesson(slotId);if(current)Object.assign(current,patch);else timetable.value.lessons.push({weekday:weekday.value,slotId,subjectId:timetable.value.subjects[0]?.id??'',enabled:false,...patch}) }
const alertText=ref('')
</script>
<template>
	<div class="vault-timetable-editor">
		<h3>科目</h3><p class="vault-muted text-sm">只记录科目，不采集教师身份。</p>
		<div class="vault-subject-editor"><div v-for="subject in timetable.subjects" :key="subject.id" class="flex gap-2"><Input v-model="subject.name" aria-label="科目名称" placeholder="科目名称" class="grow"/><IconButton label="移除科目" type="quiet" @click="removeSubject(subject.id)"><XIcon/></IconButton></div></div><Button type="outlined" @click="addSubject"><PlusIcon/>添加科目</Button>
		<h3>每日共用时间段</h3><p class="vault-muted text-sm">时间使用 HH:mm；时间段不能重叠或跨午夜。取消“上课”可定义课间或其他非课程时间。</p>
		<div v-for="slot in timetable.slots" :key="slot.id" class="vault-timeslot-row"><Input v-model="slot.label" aria-label="时间段名称" placeholder="第1节"/><Input v-model="slot.start" aria-label="开始时间" placeholder="08:00"/><span>–</span><Input v-model="slot.end" aria-label="结束时间" placeholder="08:40"/><Checkbox :model-value="slot.isClass" label="上课" @update:model-value="value=>setClass(slot.id,value)"/><IconButton label="移除时间段" type="quiet" @click="removeSlot(slot.id)"><XIcon/></IconButton></div>
		<Button type="outlined" @click="addSlot"><PlusIcon/>添加时间段</Button>
		<h3>星期课程</h3><DropdownSelect v-model="weekday" name="编辑星期" aria-label="编辑星期" :options="[1,2,3,4,5,6,7]" :display-name="day=>days[day]"/>
		<p v-if="!classSlots.length" class="vault-muted">请先添加上课时间段。</p>
		<div v-for="slot in classSlots" :key="slot.id" class="vault-weeklesson-row"><div><strong>{{slot.label}}</strong><small>{{slot.start}}–{{slot.end}}</small></div><Checkbox :model-value="lesson(slot.id)?.enabled??false" :label="`启用${slot.label}`" @update:model-value="value=>setLesson(slot.id,{enabled:value})"/><DropdownSelect :model-value="lesson(slot.id)?.subjectId||timetable.subjects[0]?.id||''" :name="`科目-${slot.id}`" :aria-label="`${slot.label}科目`" :disabled="!timetable.subjects.length" :options="timetable.subjects.map(subject=>subject.id)" :display-name="id=>timetable.subjects.find(subject=>subject.id===id)?.name||'未命名科目'" @update:model-value="value=>setLesson(slot.id,{subjectId:String(value)})"/></div>
		<p class="vault-muted text-sm">普通周课表编辑不改变历史记录；当天调课从课程卡片右键或标题长按进入，只影响当天课次。</p><p v-if="alertText" class="vault-file-error" role="status">{{alertText}}</p>
	</div>
</template>
