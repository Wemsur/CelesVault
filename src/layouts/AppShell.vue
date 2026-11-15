<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { NavButton, WindowControls, IconButton, Input } from '@modrinth/ui'
import { HomeIcon, LibraryIcon, SettingsIcon, SearchIcon, SquareSplitHorizontalIcon, ServerIcon } from '@modrinth/assets'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'

const shell = useShellStore()
const vault = useVaultStore()
const platform = usePlatform()
const route = useRoute()
const router = useRouter()
const windowControlsWidth = ref(0)
const isHome = computed(() => route.name === 'home')
function searchLibrary() {
	shell.statusMessage = '全局库搜索在后续阶段接入；当前输入不会查询或读取文件。'
	if (route.name !== 'library') void router.push('/library')
}
</script>

<template>
	<div class="vault-app vault-shell" :style="{ '--window-controls-width': `${windowControlsWidth}px` }">
		<header class="vault-topbar" data-tauri-drag-region>
			<div class="vault-brand" data-tauri-drag-region><span>CelesVault</span></div>
			<div class="vault-search" data-tauri-drag-region-exclude>
				<Input v-model="shell.search" placeholder="搜索库内文件" aria-label="搜索库内文件" @keydown.enter="searchLibrary">
					<template #leading><SearchIcon /></template>
				</Input>
			</div>
			<span v-if="platform.mode === 'preview'" class="vault-preview-label">浏览器预览</span>
			<div v-if="isHome" class="vault-direction" data-tauri-drag-region-exclude>
				<IconButton label="左右分屏" v-tooltip="'左右分屏'" aria-label="左右分屏" :aria-pressed="shell.homeDirection === 'horizontal'" :type="shell.homeDirection === 'horizontal' ? 'outlined' : 'quiet'" @click="shell.homeDirection = 'horizontal'"><SquareSplitHorizontalIcon /></IconButton>
				<IconButton label="上下分屏" v-tooltip="'上下分屏'" aria-label="上下分屏" :aria-pressed="shell.homeDirection === 'vertical'" :type="shell.homeDirection === 'vertical' ? 'outlined' : 'quiet'" @click="shell.homeDirection = 'vertical'"><SquareSplitHorizontalIcon class="rotate-90" /></IconButton>
			</div>
			<WindowControls @width-change="windowControlsWidth = $event" @window-error="shell.statusMessage = '窗口状态读取失败，未切换为模拟模式。'" />
		</header>
		<nav class="vault-navigation" aria-label="主要导航">
			<NavButton v-tooltip.right="'主页'" to="/" :is-primary="r => r.path === '/'" aria-label="主页"><HomeIcon /></NavButton>
			<NavButton v-tooltip.right="'库'" to="/library" :is-primary="r => r.path === '/library'" aria-label="库"><LibraryIcon /></NavButton>
			<hr class="vault-navigation-divider" />
			<NavButton v-for="drive in vault.drives" :key="drive.identity" v-tooltip.right="drive.name" :to="() => vault.browse(platform, drive.root)" :aria-label="drive.name"><ServerIcon /></NavButton>
			<span v-if="!vault.drives.length" v-tooltip.right="platform.mode === 'preview' ? '浏览器不读取驱动器' : '没有可浏览的驱动器'">
				<NavButton :to="() => {}" :disabled="true" aria-label="没有可浏览的驱动器"><ServerIcon /></NavButton>
			</span>
			<div class="vault-nav-bottom">
				<NavButton v-tooltip.right="'设置'" :to="() => { shell.settingsOpen = true }" aria-label="设置"><SettingsIcon /></NavButton>
			</div>
		</nav>
		<main class="vault-content">
			<span class="sr-only" role="status">{{ shell.statusMessage }}</span>
			<RouterView />
		</main>
	</div>
</template>
