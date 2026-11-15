<script setup lang="ts">
import { computed } from 'vue'
import { Card, Button, Checkbox, IconButton } from '@modrinth/ui'
import { LibraryIcon, FileTextIcon, FileImageIcon, PinIcon, CopyIcon, XIcon } from '@modrinth/assets'
import DirectoryBrowser from '@/features/file-browser/DirectoryBrowser.vue'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const shell = useShellStore()
const vault = useVaultStore()
const platform = usePlatform()
const allSelected = computed(() => vault.available.length > 0 && vault.available.every(item => vault.selected.includes(item.id)))
const imageFormats = new Set(['jpg', 'jpeg', 'png', 'gif', 'bmp', 'webp', 'tif', 'tiff', 'heic', 'heif'])
const time = (ms: number) => new Date(ms).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
async function retry(id: string) { try { await platform.vault.retry(id); await vault.refresh(platform) } catch (error) { vault.error = String(error) } }
async function open(id: string) { try { await platform.vault.openArchive(id) } catch (error) { vault.error = String(error) } }
</script>
<template>
	<section class="vault-home-page" aria-label="课程资料与导出目录">
		<div class="vault-home-panels" :class="{ vertical: shell.homeDirection === 'vertical' }" data-testid="home-panels">
			<Card class="vault-panel vault-archive-panel">
				<div class="vault-panel-title"><LibraryIcon /><h2>未分类</h2><span class="vault-muted text-xs">课表尚未配置</span><Checkbox class="ml-auto" label="选择所有可导出资料" label-class="sr-only" :model-value="allSelected" :disabled="!vault.available.length" @update:model-value="value => vault.selected = value ? vault.available.map(item => item.id) : []" /></div>
				<div v-if="!vault.items.length" class="vault-empty"><LibraryIcon /><strong>还没有收纳资料</strong><p>{{ platform.mode === 'preview' ? '浏览器仅预览界面；真实监测和文件操作请在 Tauri dev 验证。' : vault.status?.dataRoot ? '用 WPS 打开来源范围中的教学文件，成功保存后会出现在这里。' : '请先在设置中选择不受还原影响的持久目录。' }}</p><Button v-if="!vault.status?.dataRoot" type="outlined" @click="shell.settingsOpen = true">设置持久目录</Button></div>
				<div v-else class="vault-file-grid">
					<div v-for="item in vault.items" :key="item.id" class="vault-file-tile" :class="{ selected: vault.selected.includes(item.id) }" @click="item.state === 'available' && vault.select(item.id, !vault.selected.includes(item.id))" @dblclick="item.state === 'available' && open(item.id)">
						<Checkbox :label="`选择 ${item.displayName}`" label-class="sr-only" :model-value="vault.selected.includes(item.id)" :disabled="item.state !== 'available'" @click.stop @update:model-value="value => vault.select(item.id, value)" />
						<component :is="imageFormats.has(item.extension) ? FileImageIcon : FileTextIcon" class="vault-file-icon" />
						<span class="vault-file-name" :title="item.displayName">{{ item.displayName }}</span><small>{{ time(item.detectedUnixMs) }} · {{ item.state === 'available' ? '已保存' : item.state === 'failed' ? '失败' : '等待保存' }}</small>
						<Button v-if="item.state === 'failed' && item.taskId" size="sm" @click.stop="retry(item.taskId)">重试</Button><small v-if="item.error" class="vault-file-error">{{ item.error }}</small>
					</div>
				</div>
				<div v-if="vault.error" class="vault-file-error text-sm" role="status">{{ vault.error }}</div>
			</Card>
			<div class="vault-home-target">
				<Card class="vault-panel"><div class="vault-panel-title"><PinIcon /><h2>固定文件夹</h2></div><div class="vault-empty"><p>物理设备固定夹将在下一阶段接入。当前可从侧栏选择真实驱动器和目标目录。</p></div></Card>
				<Card class="vault-panel !p-0"><DirectoryBrowser /></Card>
			</div>
		</div>
		<div v-if="vault.selected.length" class="vault-copy-bar"><Button type="colored" color="brand" :disabled="!vault.target || !vault.selectedAvailable.length || vault.busy || platform.mode !== 'native'" @click="vault.exportSelected(platform)"><CopyIcon />复制全部（{{ vault.selectedAvailable.length }}）</Button><IconButton label="清除选择" @click="vault.selected = []"><XIcon /></IconButton><span class="text-xs vault-muted">同名默认保留两份</span></div>
		<div v-if="vault.jobs.length" class="vault-job-summary"><div v-for="job in vault.jobs.slice(0,3)" :key="job.id"><span>{{ job.state === 'running' ? '复制中' : job.state === 'completed' ? '复制完成' : job.state === 'cancelled' ? '已取消' : '复制失败' }} {{ job.processed }}/{{ job.total }} · 已复制 {{ job.copied }}</span><Button v-if="job.state === 'running'" size="sm" @click="platform.files.cancel(job.id).catch(error => vault.error = String(error))">取消</Button><p v-if="job.error" class="vault-file-error">{{ job.error }}</p></div></div>
	</section>
</template>
