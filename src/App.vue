<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { NewModal, Button, provideModalBehavior } from '@modrinth/ui'
import AppShell from '@/layouts/AppShell.vue'
import SettingsModal from '@/features/settings/SettingsModal.vue'
import { usePlatform } from '@/platform'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'

const shell = useShellStore()
const vault = useVaultStore()
let refreshTimer: ReturnType<typeof setInterval> | undefined
const platform = usePlatform()
const closeNotice = ref<InstanceType<typeof NewModal> | null>(null)
provideModalBehavior({ noblur: ref(false) })
const rawHide = platform.window.hide
const rawMinimize = platform.window.minimize
const rawToggle = platform.window.toggleMaximize
const reportError = (error: unknown) => { shell.statusMessage = `操作未完成：${error instanceof Error ? error.message : String(error)}` }

platform.window.hide = async () => {
	if (!shell.firstCloseAcknowledged) { closeNotice.value?.show(); return }
	try { await rawHide() } catch (error) { reportError(error) }
}
platform.window.minimize = async () => { try { await rawMinimize() } catch (error) { reportError(error) } }
platform.window.toggleMaximize = async () => { try { await rawToggle() } catch (error) { reportError(error) } }
async function confirmHide() {
	shell.firstCloseAcknowledged = true
	closeNotice.value?.hide()
	await platform.window.hide()
}
const previewMessage = (event: Event) => { shell.statusMessage = (event as CustomEvent<string>).detail }

onMounted(async () => {
	window.addEventListener('vault:preview-window-action', previewMessage)
	try { shell.runtime = await platform.app.getRuntimeInfo() } catch (error) { reportError(error) }
	await vault.refresh(platform)
	refreshTimer = setInterval(() => { void vault.refresh(platform) }, 2000)
})
onUnmounted(() => {
	if (refreshTimer) clearInterval(refreshTimer)
	window.removeEventListener('vault:preview-window-action', previewMessage)
	platform.window.hide = rawHide
	platform.window.minimize = rawMinimize
	platform.window.toggleMaximize = rawToggle
})
</script>

<template>
	<AppShell />
	<SettingsModal />
	<NewModal ref="closeNotice" header="关闭后仍在后台运行" :max-width="'28rem'">
		<p>关闭窗口不会退出 CelesVault。你可以从系统托盘重新打开，或在托盘菜单中选择“退出”。</p>
		<p class="vault-muted mt-3">监测状态可在设置或托盘中查看；未配置持久目录时不会启动收纳。</p>
		<template #actions>
			<Button type="outlined" @click="closeNotice?.hide()">取消</Button>
			<Button type="colored" color="brand" @click="confirmHide">知道了，隐藏窗口</Button>
		</template>
	</NewModal>
</template>
