<script setup lang="ts">
import { computed, nextTick, onActivated, onDeactivated, onUnmounted, ref, watch } from 'vue'
import { Button } from '@modrinth/ui'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import { useShellStore } from '@/stores/shell'
import ResizableSplit from '@/components/ResizableSplit.vue'
import CourseCard from './CourseCard.vue'
import FixedFolders from './FixedFolders.vue'
import DirectoryBrowser from '@/features/file-browser/DirectoryBrowser.vue'
const vault=useVaultStore();const platform=usePlatform();const shell=useShellStore();const now=ref(Date.now());let tick:ReturnType<typeof setInterval>|undefined
const vertical=computed(()=>vault.config.app.homeDirection==='vertical')
const ratio=computed({get:()=>vertical.value?vault.config.app.verticalRatio:vault.config.app.horizontalRatio,set:value=>{if(vertical.value)vault.config.app.verticalRatio=value;else vault.config.app.horizontalRatio=value}})
const targetRatio=computed({get:()=>vertical.value?vault.config.app.verticalTargetRatio:vault.config.app.horizontalTargetRatio,set:value=>{if(vertical.value)vault.config.app.verticalTargetRatio=value;else vault.config.app.horizontalTargetRatio=value}})
function saveRatio(){vault.persistApp(platform,vertical.value?{verticalRatio:ratio.value}:{horizontalRatio:ratio.value})}
function saveTargetRatio(){vault.persistApp(platform,vertical.value?{verticalTargetRatio:targetRatio.value}:{horizontalTargetRatio:targetRatio.value})}
const scroller=ref<HTMLElement|null>(null)
let resize:ResizeObserver|undefined
let active=false
let centeredId=''
const orderedCourses=computed(()=>[...vault.courses].sort((a,b)=>Number(a.slotId===null)-Number(b.slotId===null)||a.startUnixMs-b.startUnixMs||a.id.localeCompare(b.id)))
const currentId=computed(()=>{
	const scheduled=orderedCourses.value.filter(course=>course.slotId!==null)
	return scheduled.find(course=>now.value<course.endUnixMs)?.id??scheduled.at(-1)?.id??orderedCourses.value[0]?.id??''
})
function jump(id:string,animate=true){
	const root=scroller.value
	const card=root&&Array.from(root.querySelectorAll<HTMLElement>('[data-course-id]')).find(node=>node.dataset.courseId===id)
	if(!root||!card)return
	centeredId=id
	const cards=Array.from(root.querySelectorAll<HTMLElement>('[data-course-id]'))
	const first=card===cards[0];const last=card===cards.at(-1)
	const box=root.getBoundingClientRect();const item=card.getBoundingClientRect()
	const destination=first?0:last?(vertical.value?root.scrollWidth-root.clientWidth:root.scrollHeight-root.clientHeight):vertical.value?root.scrollLeft+item.left-box.left+(item.width-root.clientWidth)/2:root.scrollTop+item.top-box.top+(item.height-root.clientHeight)/2
	const behavior=animate&&!matchMedia('(prefers-reduced-motion: reduce)').matches?'smooth':'instant'
	root.scrollTo(vertical.value?{left:destination,behavior}:{top:destination,behavior})
}
function measure(){
	const root=scroller.value;if(!root||!root.clientHeight||!root.clientWidth)return
	root.style.setProperty('--course-viewport-width',`${root.clientWidth}px`)
	root.style.setProperty('--course-viewport-height',`${root.clientHeight}px`)
	const cards=root.querySelectorAll<HTMLElement>('[data-course-id]')
	const rem=parseFloat(getComputedStyle(root).fontSize)
	for(const card of cards){
		const tileHeight=11.5*rem
		const chrome=(card.querySelector('header')?.getBoundingClientRect().height??48)+5*rem+(card.querySelector('footer')?.getBoundingClientRect().height??0)
		const rows=Math.max(1,Math.floor((root.clientHeight-chrome)/(tileHeight+.75*rem)))
		card.style.setProperty('--course-columns',String(Math.min(3,Math.max(1,Math.ceil(Number(card.dataset.fileCount??0)/rows)))))
	}
}
function wheel(event:WheelEvent){
	if(!vertical.value||event.ctrlKey||Math.abs(event.deltaX)>=Math.abs(event.deltaY))return
	const root=scroller.value;if(!root)return
	const inner=(event.target as HTMLElement).closest('.vault-course-files')
	if(inner&&inner.scrollHeight>inner.clientHeight&&((event.deltaY>0&&inner.scrollTop+inner.clientHeight<inner.scrollHeight-1)||(event.deltaY<0&&inner.scrollTop>0)))return
	event.preventDefault();root.scrollBy({left:event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?root.clientWidth:1),behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'})
}
function rememberCenter(){
	const root=scroller.value;if(!root)return
	const position=vertical.value?root.scrollLeft:root.scrollTop
	const maximum=vertical.value?root.scrollWidth-root.clientWidth:root.scrollHeight-root.clientHeight
	if(position<=2){centeredId=orderedCourses.value[0]?.id??'';return}
	if(position>=maximum-2){centeredId=orderedCourses.value.at(-1)?.id??'';return}
	const bounds=root.getBoundingClientRect();const center=vertical.value?bounds.left+root.clientWidth/2:bounds.top+root.clientHeight/2
	let distance=Infinity
	for(const card of root.querySelectorAll<HTMLElement>('[data-course-id]')){const box=card.getBoundingClientRect();const delta=Math.abs((vertical.value?box.left+box.width/2:box.top+box.height/2)-center);if(delta<distance){distance=delta;centeredId=card.dataset.courseId??''}}
}
// Polling and clock changes never reposition a view the user is browsing.
watch(()=>orderedCourses.value.map(course=>course.id).join('|'),async(value,previous)=>{await nextTick();if(active){measure();if(!previous&&value)jump(currentId.value,false)}})
watch(vertical,async()=>{await nextTick();if(active){measure();jump(centeredId||currentId.value,false)}})
watch(()=>`${vault.config.app.scalePercent}|${orderedCourses.value.map(course=>`${course.id}:${course.items.length}`).join('|')}`,async()=>{await nextTick();if(active){measure();requestAnimationFrame(()=>{if(active)measure()})}})
async function activate(){active=true;vault.targetContext='home';now.value=Date.now();if(tick)clearInterval(tick);tick=setInterval(()=>now.value=Date.now(),10000);await nextTick();if(!active)return;resize?.disconnect();resize=new ResizeObserver(measure);if(scroller.value)resize.observe(scroller.value);measure();jump(currentId.value,false)}
function deactivate(){active=false;if(tick)clearInterval(tick);tick=undefined;resize?.disconnect()}
onActivated(activate)
onDeactivated(deactivate)
onUnmounted(deactivate)
</script>
<template>
	<section class="vault-home-page" aria-label="课程资料与导出目录"><ResizableSplit v-model="ratio" :vertical="vertical" :min-size="220" @commit="saveRatio">
		<template #first><div ref="scroller" class="vault-courses" :class="{horizontal:vertical,'has-courses':orderedCourses.length>0}" data-testid="course-scroll" tabindex="0" aria-label="按课表顺序排列的课程，可滚动切换" @wheel="wheel" @scrollend="rememberCenter"><CourseCard v-for="course in orderedCourses" :key="course.id" :course="course" :now="now" @jump="id=>jump(id)"/><div v-if="!vault.courses.length" class="vault-empty"><strong>{{vault.status?.dataRoot?'还没有课程或资料':'先选择持久目录'}}</strong><p>{{platform.mode==='preview'?'浏览器仅预览界面，不执行真实文件操作。':'课表可稍后配置，暂无课表时资料归入未分类。'}}</p><Button type="outlined" @click="shell.settingsOpen=true">打开设置</Button></div></div></template>
		<template #second><ResizableSplit v-model="targetRatio" :vertical="!vertical" :min-size="150" @commit="saveTargetRatio"><template #first><FixedFolders/></template><template #second><DirectoryBrowser/></template></ResizableSplit></template>
	</ResizableSplit></section>
</template>
