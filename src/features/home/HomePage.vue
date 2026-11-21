<script setup lang="ts">
import { computed, onActivated, onDeactivated, onMounted, onUnmounted, ref } from 'vue'
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
function jump(id:string){document.getElementById(`course-${id}`)?.scrollIntoView({behavior:'smooth',block:'nearest',inline:'nearest'})}
function activate(){vault.targetContext='home';now.value=Date.now();if(tick)clearInterval(tick);tick=setInterval(()=>now.value=Date.now(),10000)}
function deactivate(){if(tick)clearInterval(tick);tick=undefined}
onMounted(activate)
onActivated(activate)
onDeactivated(deactivate)
onUnmounted(deactivate)
</script>
<template>
	<section class="vault-home-page" aria-label="课程资料与导出目录"><ResizableSplit v-model="ratio" :vertical="vertical" :min-size="220" @commit="saveRatio">
		<template #first><div class="vault-courses" :class="{horizontal:vertical}" data-testid="course-scroll"><CourseCard v-for="course in vault.courses" :key="course.id" :course="course" :now="now" @jump="jump"/><div v-if="!vault.courses.length" class="vault-empty"><strong>{{vault.status?.dataRoot?'还没有课程或资料':'先选择持久目录'}}</strong><p>{{platform.mode==='preview'?'浏览器仅预览界面，不执行真实文件操作。':'课表可稍后配置，暂无课表时资料归入未分类。'}}</p><Button type="outlined" @click="shell.settingsOpen=true">打开设置</Button></div></div></template>
		<template #second><ResizableSplit v-model="targetRatio" :vertical="!vertical" :min-size="150" @commit="saveTargetRatio"><template #first><FixedFolders/></template><template #second><DirectoryBrowser/></template></ResizableSplit></template>
	</ResizableSplit></section>
</template>
