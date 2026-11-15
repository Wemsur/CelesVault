<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Input, IconButton, Button } from '@modrinth/ui'
import { FolderIcon, FileIcon, ChevronLeftIcon, RefreshCwIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const vault = useVaultStore()
const platform = usePlatform()
const address = ref('')
const page = ref(0)
watch(() => vault.target?.path, value => { address.value = value ?? ''; page.value = 0 })
const entries = computed(() => (vault.target?.entries ?? []).slice(page.value * 200, (page.value + 1) * 200))
const parent = computed(() => {
	const path = vault.target?.path.replace(/\\+$/, '')
	if (!path || path.length <= 2) return null
	const at = path.lastIndexOf('\\')
	return at <= 2 ? path.slice(0, 2) + '\\' : path.slice(0, at)
})
const size = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`
</script>
<template>
	<div class="vault-browser">
		<div class="vault-browser-toolbar">
			<IconButton label="上级目录" type="quiet" :disabled="!parent || vault.busy" @click="parent && vault.browse(platform, parent)"><ChevronLeftIcon /></IconButton>
			<Input v-model="address" placeholder="选择侧栏驱动器或输入本地路径" aria-label="目标目录路径" class="grow min-w-0" @keydown.enter="vault.browse(platform, address)" />
			<IconButton label="刷新目录" type="quiet" :disabled="!vault.target || vault.busy" @click="vault.target && vault.browse(platform, vault.target.path)"><RefreshCwIcon /></IconButton>
		</div>
		<div v-if="!vault.target" class="vault-empty"><FolderIcon /><strong>未选择目标目录</strong><p>点击左侧本地盘或 U 盘。浏览器预览不会读取真实设备。</p></div>
		<div v-else class="vault-directory-list">
			<button v-for="entry in entries" :key="entry.path" class="vault-directory-entry" :disabled="!entry.isDir" @dblclick="entry.isDir && vault.browse(platform, entry.path)">
				<component :is="entry.isDir ? FolderIcon : FileIcon" /><span>{{ entry.name }}</span><small>{{ entry.isDir ? '文件夹 · 双击进入' : size(entry.size) }}</small>
			</button>
			<p v-if="!vault.target.entries.length" class="vault-muted p-4">此文件夹为空</p>
		</div>
		<div v-if="vault.target && vault.target.entries.length > 200" class="flex items-center gap-2 p-2"><Button :disabled="page === 0" @click="page--">上一页</Button><span>{{ page + 1 }}</span><Button :disabled="(page + 1) * 200 >= vault.target.entries.length" @click="page++">下一页</Button></div>
	</div>
</template>
