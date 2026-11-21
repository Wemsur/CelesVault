<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useAppearance } from '@/platform/appearance'
import { BackgroundRenderer } from './background-renderer'
import { backgroundMessage } from './background-status'
const {settings,dark}=useAppearance()
const host=ref<HTMLElement|null>(null);const canvas=ref<HTMLCanvasElement|null>(null)
let renderer:BackgroundRenderer|null=null
const active=computed(()=>settings.value.background!=='off')
const style=computed(()=>settings.value.background==='image'?{
	backgroundImage:settings.value.image?`url("${settings.value.image}")`:undefined,
	backgroundSize:settings.value.imageFit,opacity:settings.value.imageOpacity,filter:`blur(${settings.value.imageBlur}px)`,
}:{opacity:settings.value.parameters[settings.value.shader].intensity})
function sync(){
	if(!host.value||!canvas.value)return
	document.documentElement.classList.toggle('vault-background-enabled',active.value)
	if(settings.value.background!=='animated'){renderer?.dispose();renderer=null;backgroundMessage.value='';return}
	renderer??=new BackgroundRenderer(canvas.value,host.value,message=>{backgroundMessage.value=message})
	renderer.apply(settings.value.shader,settings.value.parameters[settings.value.shader],dark.value)
}
watch([settings,dark],sync,{deep:true})
onMounted(sync)
onUnmounted(()=>{renderer?.dispose();document.documentElement.classList.remove('vault-background-enabled')})
</script>
<template>
	<div v-show="active" class="vault-background" aria-hidden="true" :data-background="settings.background" :data-shader="settings.shader">
		<div ref="host" class="vault-background-art" :style="style"><canvas v-show="settings.background==='animated'" ref="canvas" /></div>
	</div>
</template>
