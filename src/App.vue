<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { NewModal, Button, provideModalBehavior, provideNotificationManager } from '@modrinth/ui'
import { AppNotificationManager } from '@/platform/notifications'
import OperationNotifications from '@/components/OperationNotifications.vue'
import AppShell from '@/layouts/AppShell.vue'
import SettingsModal from '@/features/settings/SettingsModal.vue'
import CopyActionBar from '@/components/CopyActionBar.vue'
import CopyConflictModal from '@/components/CopyConflictModal.vue'
import ArchiveDetailModal from '@/components/ArchiveDetailModal.vue'
import SafeEject from '@/components/SafeEject.vue'
import BackgroundLayer from '@/features/appearance/BackgroundLayer.vue'
import { initializeAppearance } from '@/platform/appearance'
import { usePlatform, type Unlisten } from '@/platform'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'

const shell = useShellStore()
const vault = useVaultStore()
const platform = usePlatform()
const stopAppearance = initializeAppearance()
const closeNotice = ref<InstanceType<typeof NewModal> | null>(null)
provideModalBehavior({ noblur: ref(false) })
const notifications = new AppNotificationManager(() => vault.config.app.notificationSeconds)
provideNotificationManager(notifications)
let refreshTimer: ReturnType<typeof setInterval> | undefined
let unlistenConfig: Unlisten | undefined
let disposed = false
const original = { hide: platform.window.hide, minimize: platform.window.minimize, toggle: platform.window.toggleMaximize }
const reportError = (failure: unknown) => { shell.statusMessage = `操作未完成：${String(failure)}`; vault.error = shell.statusMessage }
platform.window.hide = async () => {
	if (!shell.firstCloseAcknowledged && !vault.config.app.closeNoticed) { closeNotice.value?.show(); return }
	try { await original.hide() } catch (failure) { reportError(failure) }
}
platform.window.minimize = async () => { try { await original.minimize() } catch (failure) { reportError(failure) } }
platform.window.toggleMaximize = async () => { try { await original.toggle() } catch (failure) { reportError(failure) } }
async function confirmHide() {
	shell.firstCloseAcknowledged = true
	if (platform.mode === 'native' && vault.status?.dataRoot) {
		try { vault.config = await platform.vault.saveApp({ ...vault.config.app, closeNoticed: true }) } catch (failure) { reportError(failure) }
	}
	closeNotice.value?.hide()
	await platform.window.hide()
}
const previewMessage = (event: Event) => { shell.statusMessage = (event as CustomEvent<string>).detail }
watch(() => vault.config.app.scalePercent, value => { document.documentElement.style.fontSize = `${16 * value / 100}px` }, { immediate: true })
onMounted(async () => {
	window.addEventListener('vault:preview-window-action', previewMessage)
	try { shell.runtime = await platform.app.getRuntimeInfo() } catch (failure) { reportError(failure) }
	await vault.refresh(platform)
	if (disposed) return
	try { unlistenConfig = await platform.vault.onConfigChanged(() => { void vault.loadConfig(platform).catch(reportError) }); if (disposed) unlistenConfig() } catch (failure) { reportError(failure) }
	if (!disposed) refreshTimer = setInterval(() => { void vault.refresh(platform) }, 2000)
})
onUnmounted(() => {
	disposed = true
	stopAppearance()
	if (refreshTimer) clearInterval(refreshTimer)
	unlistenConfig?.()
	window.removeEventListener('vault:preview-window-action', previewMessage)
	platform.window.hide = original.hide; platform.window.minimize = original.minimize; platform.window.toggleMaximize = original.toggle
})
</script>
<template>
	<BackgroundLayer />
	<AppShell />
	<CopyActionBar />
	<OperationNotifications :manager="notifications" />
	<SafeEject />
	<SettingsModal />
	<CopyConflictModal />
	<ArchiveDetailModal />
	<NewModal ref="closeNotice" header="关闭后仍在后台运行" max-width="28rem">
		<p>关闭窗口不会退出 CelesVault。你可以从系统托盘重新打开，或在托盘菜单中选择“退出”。</p>
		<p class="vault-muted mt-3">监测状态可在设置或托盘中查看；未配置持久目录时不会启动收纳。</p>
		<template #actions><Button type="outlined" @click="closeNotice?.hide()">取消</Button><Button type="colored" color="brand" @click="confirmHide">知道了，隐藏窗口</Button></template>
	</NewModal>
</template>
