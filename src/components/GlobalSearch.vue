<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { Input, Button } from '@modrinth/ui'
import { SearchIcon } from '@modrinth/assets'
import { usePlatform } from '@/platform'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { useRouter } from 'vue-router'
import type { ArchiveItem, LibraryQuery } from '@/platform/vault-types'
const shell=useShellStore();const vault=useVaultStore();const platform=usePlatform();const router=useRouter()
const focused=ref(false);const results=ref<ArchiveItem[]>([]);const error=ref('');const regex=ref(false);let timer:ReturnType<typeof setTimeout>|undefined;let request=0
const visible=computed(()=>focused.value&&shell.search.length>0)
watch([()=>shell.search,regex],()=>{if(timer)clearTimeout(timer);const token=++request;timer=setTimeout(async()=>{error.value='';try{const query:LibraryQuery={name:shell.search,regex:regex.value,subjects:[],dateFrom:null,dateTo:null,extensions:[],groupBy:'none',sortBy:'used',descending:true,offset:0,limit:8};const response=await platform.vault.library(query);if(token===request)results.value=response.rows.map(row=>row.item)}catch(e){if(token===request){error.value=String(e);results.value=[]}}},180)})
const active = ref(-1)
function enter(){focused.value=false;void router.push({path:'/library',query:{q:shell.search,...(regex.value?{regex:'1'}:{})}})}
function keydown(event: KeyboardEvent){if(event.key==='Escape'){focused.value=false;active.value=-1}else if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();focused.value=true;const step=event.key==='ArrowDown'?1:-1;active.value=results.value.length?(active.value+step+results.value.length)%results.value.length:-1}else if(event.key==='Enter'){event.preventDefault();const item=results.value[active.value];if(item){void vault.showDetail(platform,item.id);focused.value=false}else enter()}}
let blurTimer: ReturnType<typeof setTimeout> | undefined
function blur(){blurTimer=setTimeout(()=>{focused.value=false},200)}
watch(()=>shell.search,()=>{active.value=-1})
onUnmounted(()=>{if(timer)clearTimeout(timer);if(blurTimer)clearTimeout(blurTimer);request++})
</script>
<template>
	<div class="vault-search-control" @focusin="focused=true" @focusout="blur"><Input v-model="shell.search" placeholder="搜索库内文件" aria-label="搜索库内文件" clearable @keydown="keydown"><template #leading><SearchIcon/></template></Input><Button type="quiet" size="sm" class="vault-regex-toggle" :aria-pressed="regex" title="显式正则搜索" @click="regex=!regex">.*</Button>
		<div v-if="visible" class="vault-search-results"><p v-if="error" class="vault-file-error p-3" role="status">{{error}}</p><Button v-for="(item,index) in results" :key="item.id" type="quiet" class="w-full justify-start" :class="{'vault-search-active':active===index}" @click="vault.showDetail(platform,item.id);focused=false"><span class="truncate">{{item.displayName}}</span></Button><p v-if="!results.length&&!error" class="vault-muted p-3 text-sm">没有匹配资料</p><Button type="outlined" class="w-full" @click="enter">在库中查看结果</Button></div>
	</div>
</template>
