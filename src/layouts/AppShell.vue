<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RouterView, useRoute } from 'vue-router'
import { NavButton, WindowControls, IconButton } from '@modrinth/ui'
import { HomeIcon, LibraryIcon, SettingsIcon, SquareSplitHorizontalIcon, ServerIcon, PlugIcon } from '@modrinth/assets'
import GlobalSearch from '@/components/GlobalSearch.vue'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'

const shell = useShellStore()
const vault = useVaultStore()
const platform = usePlatform()
const route = useRoute()
const windowControlsWidth = ref(0)
const isHome = computed(() => route.name === 'home')
function direction(value: 'horizontal' | 'vertical') { vault.persistApp(platform, { homeDirection: value }) }
watch(() => route.name, name => {
	vault.targetContext = name === 'library' ? 'library' : 'home'
	if (platform.mode === 'native') {
		const target = vault.target
		const action = target ? platform.files.watch(target.path, target.identity) : platform.files.unwatch()
		void action.catch(failure => { vault.error = String(failure) })
	}
}, { immediate: true })
</script>
<template>
	<div class="vault-app vault-shell" :style="{ '--window-controls-width': `${windowControlsWidth}px` }">
		<header class="vault-topbar" data-tauri-drag-region>
			<div class="vault-brand" data-tauri-drag-region><span>CelesVault</span></div>
			<div class="vault-search" data-tauri-drag-region-exclude><GlobalSearch /></div>
			<span v-if="platform.mode === 'preview'" class="vault-preview-label">浏览器预览</span>
			<div v-if="isHome" class="vault-direction" data-tauri-drag-region-exclude><IconButton label="左右分屏" :aria-pressed="vault.config.app.homeDirection === 'horizontal'" :type="vault.config.app.homeDirection === 'horizontal' ? 'outlined' : 'quiet'" @click="direction('horizontal')"><SquareSplitHorizontalIcon /></IconButton><IconButton label="上下分屏" :aria-pressed="vault.config.app.homeDirection === 'vertical'" :type="vault.config.app.homeDirection === 'vertical' ? 'outlined' : 'quiet'" @click="direction('vertical')"><SquareSplitHorizontalIcon class="rotate-90" /></IconButton></div>
			<WindowControls @width-change="windowControlsWidth = $event" @window-error="vault.error = `窗口状态读取失败：${String($event)}`" />
		</header>
		<nav class="vault-navigation" aria-label="主要导航"><NavButton v-tooltip.right="'主页'" to="/" :is-primary="current => current.path === '/'" aria-label="主页"><HomeIcon /></NavButton><NavButton v-tooltip.right="'库'" to="/library" :is-primary="current => current.path === '/library'" aria-label="库"><LibraryIcon /></NavButton><hr class="vault-navigation-divider" />
			<div class="vault-drive-navigation"><NavButton v-for="drive in vault.drives" :key="drive.identity" v-tooltip.right="drive.name" :to="() => vault.browse(platform, drive.root)" :aria-label="drive.name"><component :is="drive.isUsb ? PlugIcon : ServerIcon" /></NavButton><span v-if="!vault.drives.length" v-tooltip.right="platform.mode === 'preview' ? '浏览器不读取驱动器' : '没有可浏览的驱动器'"><NavButton :to="() => {}" :disabled="true" aria-label="没有可浏览的驱动器"><ServerIcon /></NavButton></span></div>
			<div class="vault-nav-bottom"><NavButton v-tooltip.right="'设置'" :to="() => { shell.settingsOpen = true }" aria-label="设置"><SettingsIcon /></NavButton></div>
		</nav>
		<main class="vault-content"><span class="sr-only" role="status">{{ shell.statusMessage || vault.error }}</span><RouterView v-slot="{ Component }"><KeepAlive><component :is="Component" /></KeepAlive></RouterView></main>
	</div>
</template>
