<script setup lang="ts">
import { Button } from '@modrinth/ui'
import { CopyIcon, XIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const vault = useVaultStore(); const platform = usePlatform()
</script>
<template>
	<div v-if="vault.selected.length" class="vault-copy-bar">
		<Button type="colored" color="brand" :disabled="!vault.target?.writable || !vault.selectedAvailable.length || vault.busy || platform.mode !== 'native'" @click="vault.requestExport(platform,vault.selected)"><CopyIcon />复制全部（{{ vault.selectedAvailable.length }}）</Button>
		<Button type="quiet" @click="vault.clearSelection"><XIcon />清除</Button>
		<span v-if="!vault.target" class="text-xs vault-muted">请选择目标目录</span>
	</div>
	<div v-if="vault.pointerDrag" class="vault-drag-ghost" :style="{left:`${vault.pointerDrag.x + 16}px`,top:`${vault.pointerDrag.y + 16}px`}"><CopyIcon />复制 {{ vault.pointerDrag.count }} 份</div>
</template>
