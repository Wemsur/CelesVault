<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { Input, Button, NewModal } from '@modrinth/ui'
import { SearchIcon } from '@modrinth/assets'
import { usePlatform } from '@/platform'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { useLibraryStore } from '@/stores/library'
import { useRoute, useRouter } from 'vue-router'
import type { ArchiveItem, LibraryQuery } from '@/platform/vault-types'
const shell=useShellStore();const vault=useVaultStore();const library=useLibraryStore();const platform=usePlatform();const router=useRouter();const route=useRoute()
const focused=ref(false);const results=ref<ArchiveItem[]>([]);const error=ref('');let timer:ReturnType<typeof setTimeout>|undefined;let request=0
const tutorial=ref<InstanceType<typeof NewModal>|null>(null)
const visible=computed(()=>focused.value&&shell.search.length>0)
const active=ref(-1)
// The persistent top bar owns search state; route changes restore deep-linked searches.
watch(()=>[route.name,route.query.q,route.query.regex],()=>{
	if(route.name!=='library')return
	if(route.query.q!==undefined){shell.search=String(route.query.q);shell.searchRegex=route.query.regex==='1'}
	library.search(shell.search,shell.searchRegex)
},{immediate:true})
watch([()=>shell.search,()=>shell.searchRegex],()=>{
	active.value=-1
	if(route.name==='library')library.search(shell.search,shell.searchRegex)
	if(timer)clearTimeout(timer)
	const token=++request
	if(!shell.search){results.value=[];error.value='';return}
	timer=setTimeout(async()=>{
		error.value=''
		try{const query:LibraryQuery={name:shell.search,regex:shell.searchRegex,subjects:[],dateFrom:null,dateTo:null,extensions:[],groupBy:'none',sortBy:'used',descending:true,offset:0,limit:8};const response=await platform.vault.library(query);if(token===request)results.value=response.rows.map(row=>row.item)}
		catch(e){if(token===request){error.value=String(e);results.value=[]}}
	},180)
})
function enter(){focused.value=false;library.search(shell.search,shell.searchRegex);void router.push({path:'/library',query:{q:shell.search,...(shell.searchRegex?{regex:'1'}:{})}})}
function keydown(event:KeyboardEvent){if(event.key==='Escape'){focused.value=false;active.value=-1}else if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();focused.value=true;const step=event.key==='ArrowDown'?1:-1;active.value=results.value.length?(active.value+step+results.value.length)%results.value.length:-1}else if(event.key==='Enter'){event.preventDefault();const item=results.value[active.value];if(item){void vault.showDetail(platform,item.id);focused.value=false}else enter()}}
let blurTimer:ReturnType<typeof setTimeout>|undefined
function blur(event:FocusEvent){if((event.currentTarget as HTMLElement).contains(event.relatedTarget as Node|null))return;blurTimer=setTimeout(()=>{focused.value=false},200)}
function focus(){if(blurTimer)clearTimeout(blurTimer);focused.value=true}
let pressTimer:ReturnType<typeof setTimeout>|undefined
let pressPointer:number|null=null
let origin={x:0,y:0}
let suppressClick=false
function cancelPress(){if(pressTimer)clearTimeout(pressTimer);pressTimer=undefined;pressPointer=null}
function showTutorial(){cancelPress();focused.value=false;tutorial.value?.show()}
function press(event:PointerEvent){if(!event.isPrimary||event.button!==0)return;cancelPress();suppressClick=false;pressPointer=event.pointerId;origin={x:event.clientX,y:event.clientY};(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);pressTimer=setTimeout(()=>{suppressClick=true;showTutorial()},600)}
function move(event:PointerEvent){if(event.pointerId===pressPointer&&Math.hypot(event.clientX-origin.x,event.clientY-origin.y)>10){suppressClick=true;cancelPress()}}
function toggleRegex(){cancelPress();if(suppressClick){suppressClick=false;return}shell.searchRegex=!shell.searchRegex}
onUnmounted(()=>{if(timer)clearTimeout(timer);if(blurTimer)clearTimeout(blurTimer);cancelPress();request++})
const lessons=[
	{pattern:'讲义',meaning:'名称中包含“讲义”'},
	{pattern:'^数学',meaning:'名称以“数学”开头'},
	{pattern:'复习.*答案',meaning:'“复习”之后出现“答案”，中间可以有其他字符'},
	{pattern:'\\.pdf$',meaning:'名称以 .pdf 结尾；反斜杠让点号表示真正的点'},
	{pattern:'\\.(pdf|pptx)$',meaning:'只匹配 PDF 或 PPTX 文件名'},
	{pattern:'第[0-9]+课',meaning:'“第”和“课”之间有一个或多个数字'},
]
</script>
<template>
	<div class="vault-search-control" @focusin="focus" @focusout="blur">
		<Input v-model="shell.search" placeholder="搜索库内文件" aria-label="搜索库内文件" clearable @keydown="keydown">
			<template #leading><SearchIcon/></template>
			<template #trailing><button type="button" class="vault-regex-toggle" :class="{active:shell.searchRegex}" :aria-pressed="shell.searchRegex" aria-label="正则表达式搜索；长按查看教学，键盘按F1查看" title="正则表达式：点击切换，长按查看教学" @click.stop="toggleRegex" @pointerdown.stop="press" @pointermove="move" @pointerup="cancelPress" @pointercancel="suppressClick=true;cancelPress()" @lostpointercapture="cancelPress" @contextmenu.prevent @keydown.stop @keydown.f1.prevent="showTutorial"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="5" cy="18" r="1.2" fill="currentColor" stroke="none"/><path d="M16 4v12M10.8 7l10.4 6M10.8 13l10.4-6"/></svg></button></template>
		</Input>
		<div v-if="visible" class="vault-search-results"><p v-if="error" class="vault-file-error p-3" role="status">{{error}}</p><Button v-for="(item,index) in results" :key="item.id" type="quiet" class="w-full justify-start" :class="{'vault-search-active':active===index}" @click="vault.showDetail(platform,item.id);focused=false"><span class="truncate">{{item.displayName}}</span></Button><p v-if="!results.length&&!error" class="vault-muted p-3 text-sm">没有匹配资料</p><Button type="outlined" class="w-full" @click="enter">在库中查看结果</Button></div>
	</div>
	<NewModal ref="tutorial" header="正则表达式入门" width="38rem" scrollable>
		<div class="vault-regex-lesson"><p>普通搜索直接输入关键词。开启 <strong>.*</strong> 后，可以用规则组合匹配文件名；搜索忽略英文字母大小写，不读取文件正文。</p><dl><template v-for="lesson in lessons" :key="lesson.pattern"><dt><code>{{lesson.pattern}}</code></dt><dd>{{lesson.meaning}}</dd></template></dl><p><code>.</code> 表示任意字符，<code>*</code> 表示重复零次或多次，<code>+</code> 表示至少一次，<code>|</code> 表示“或者”，<code>^</code> 和 <code>$</code> 表示名称开头与结尾。</p><p class="vault-muted">规则最长512字节。使用受限 Rust regex 引擎，不支持前后查找（如 (?=…)）和反向引用（如 \1）。规则有误时会在搜索结果中显示原因，不会切回普通搜索。</p></div>
		<template #actions><Button type="outlined" @click="tutorial?.hide()">知道了</Button></template>
	</NewModal>
</template>
