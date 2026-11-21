<script setup lang="ts">
import { computed, ref } from 'vue'
import { Button, Card, Checkbox, DropdownSelect } from '@modrinth/ui'
import AppearanceThemeSelector from '@/vendor/celestial/ui/components/base/AppearanceThemeSelector.vue'
import { backgroundIds, defaultBackgroundParameters, type AppearanceSettings, type BackgroundParameters, type ThemeMode } from '@/platform/appearance-types'
import { prepareBackgroundImage } from '@/platform/appearance'
import { SHADER_NAMES, isBuilder } from '@/vendor/margin-background/shaders'
import { backgroundMessage } from '@/features/appearance/background-status'
const model=defineModel<AppearanceSettings>({required:true})
const loading=ref(false);const error=ref('')
function removeImage(){model.value.image=null;if(model.value.background==='image')model.value.background='off'}
const parameters=computed(()=>model.value.parameters[model.value.shader])
const themes:ThemeMode[]=['customlight','customdark','system']
const systemTheme:ThemeMode=matchMedia('(prefers-color-scheme: dark)').matches?'customdark':'customlight'
function number(key:keyof BackgroundParameters,event:Event){const value=Number((event.target as HTMLInputElement).value);Object.assign(parameters.value,{[key]:value})}
async function choose(event:Event){
	const input=event.target as HTMLInputElement;const file=input.files?.[0];if(!file)return
	loading.value=true;error.value=''
	try{const image=await prepareBackgroundImage(file);model.value.image=image;model.value.background='image'}catch(e){error.value=String(e)}finally{loading.value=false;input.value=''}
}
function reset(){model.value.parameters[model.value.shader]=defaultBackgroundParameters()}
</script>
<template>
	<section class="vault-appearance">
		<h2>外观</h2><p class="vault-muted">沿用 Celestial 的浅深主题与自定义颜色。修改即时预览，保存后持久化；取消会恢复。</p>
		<AppearanceThemeSelector v-model="model.theme" aria-label="主题模式" :theme-options="themes" :system-theme-color="systemTheme" preferred-dark-theme="customdark" />
		<label class="vault-range-label">主题色相 <output>{{model.hue}}°</output><input v-model.number="model.hue" class="vault-hue-range" type="range" min="0" max="360" step="1" aria-label="主题色相" /></label>
		<h3>背景</h3><DropdownSelect v-model="model.background" name="背景类型" aria-label="背景类型" :options="model.image?['off','animated','image']:['off','animated']" :display-name="value=>({off:'关闭背景',animated:'动态背景',image:'本地图片'})[value]??value" />
		<template v-if="model.background==='animated'">
			<p class="vault-muted text-sm">选择背景后，下方显示该款参数，主界面同步预览。</p><div class="vault-background-options" role="group" aria-label="动态背景样式"><Button v-for="id in backgroundIds" :key="id" :type="model.shader===id?'outlined':'quiet'" :aria-pressed="model.shader===id" @click="model.shader=id">{{SHADER_NAMES[id]}}{{id==='flux'?'（WebGL版）':''}}</Button></div>
			<Card class="vault-background-parameters"><div class="flex justify-between items-center gap-2"><h3>{{SHADER_NAMES[model.shader]}}参数</h3><Button size="sm" type="quiet" @click="reset">恢复默认</Button></div>
				<label class="vault-range-label">渲染精细度 <output>{{Math.round(parameters.quality*100)}}%</output><input :value="parameters.quality" type="range" min=".25" max="1" step=".05" aria-label="渲染精细度" @input="number('quality',$event)" /></label>
				<label class="vault-range-label">动态速度 <output>{{parameters.speed.toFixed(1)}}×</output><input :value="parameters.speed" type="range" min="0" max="3" step=".1" aria-label="动态速度" @input="number('speed',$event)" /></label>
				<label class="vault-range-label">背景强度 <output>{{Math.round(parameters.intensity*100)}}%</output><input :value="parameters.intensity" type="range" min="0" max="1" step=".05" aria-label="背景强度" @input="number('intensity',$event)" /></label>
				<DropdownSelect v-model="parameters.fps" name="帧率上限" aria-label="背景帧率上限" :options="[15,30,60]" :display-name="value=>`${value} 帧/秒`" />
				<template v-if="isBuilder(model.shader)">
					<label class="vault-range-label">纹理尺度 <output>{{parameters.scale.toFixed(1)}}×</output><input :value="parameters.scale" type="range" min=".5" max="3" step=".1" aria-label="纹理尺度" @input="number('scale',$event)" /></label>
					<label v-if="model.shader==='smoke'||model.shader==='neuro'" class="vault-range-label">纹理细节 <output>{{parameters.detail.toFixed(1)}}×</output><input :value="parameters.detail" type="range" min=".5" max="3" step=".1" aria-label="纹理细节" @input="number('detail',$event)" /></label>
					<label class="vault-range-label">颗粒强度 <output>{{parameters.grain.toFixed(1)}}×</output><input :value="parameters.grain" type="range" min="0" max="3" step=".1" aria-label="颗粒强度" @input="number('grain',$event)" /></label>
				</template>
				<template v-if="model.shader==='gate'"><label class="vault-range-label">汇流线路 <output>{{parameters.density}}</output><input :value="parameters.density" type="range" min="20" max="160" step="10" aria-label="汇流线路" @input="number('density',$event)" /></label><Checkbox v-model="parameters.shockwave" label="点击产生冲击波" /></template>
				<p class="vault-muted text-sm">参数按背景独立保留。默认30帧、50%精细度；高分辨率仍受像素预算限制。窗口隐藏或系统减少动态效果时暂停动画。</p>
			</Card>
		</template>
		<h3>本地背景图片</h3><div class="flex flex-wrap gap-2 items-center"><label class="vault-file-import">{{loading?'正在处理图片…':'选择图片'}}<input type="file" accept="image/png,image/jpeg,image/webp" :disabled="loading" aria-label="选择背景图片" @change="choose" /></label><Button v-if="model.image" type="outlined" @click="removeImage">移除图片</Button></div>
		<p class="vault-muted text-sm">PNG、JPEG、WebP，原图最多12MiB。保存最长边1600像素、最多320KiB的显示副本，原图不修改；配置导出包含显示副本。</p>
		<template v-if="model.background==='image'&&model.image"><img class="vault-background-image-preview" :src="model.image" alt="已选择的背景图片" /><DropdownSelect v-model="model.imageFit" name="图片填充" aria-label="图片填充" :options="['cover','contain']" :display-name="value=>value==='cover'?'填满并裁切':'完整显示'" /><label class="vault-range-label">图片强度 <output>{{Math.round(model.imageOpacity*100)}}%</output><input v-model.number="model.imageOpacity" type="range" min="0" max="1" step=".05" aria-label="图片强度" /></label><label class="vault-range-label">图片模糊 <output>{{model.imageBlur}}px</output><input v-model.number="model.imageBlur" type="range" min="0" max="30" step="1" aria-label="图片模糊" /></label></template>
		<p v-if="error||backgroundMessage" class="vault-file-error" role="status">{{error||backgroundMessage}}</p>
	</section>
</template>
