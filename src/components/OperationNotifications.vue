<script setup lang="ts">
import { onUnmounted, watch } from 'vue'
import { NotificationPanel } from '@modrinth/ui'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { CopyJob } from '@/platform/vault-types'
import type { AppNotificationManager } from '@/platform/notifications'

const props = defineProps<{ manager: AppNotificationManager }>()
const vault = useVaultStore()
const platform = usePlatform()
const phases = new Map<string, CopyJob['state']>()
let initialized = false
function jobNotice(job: CopyJob) {
	const running = job.state === 'running'
	const type = job.state === 'failed' ? 'error' : job.state === 'cancelled' || job.skipped > 0 ? 'warning' : running ? 'info' : 'success'
	const title = running ? '正在复制资料' : job.state === 'failed' ? '复制失败' : job.state === 'cancelled' ? '复制已取消' : job.skipped > 0 ? '复制完成，部分已跳过' : '复制完成'
	const text = `已处理 ${job.processed}/${job.total}，已复制 ${job.copied}，跳过 ${job.skipped}${job.error ? `\n${job.error}` : ''}`
	props.manager.updateJob({
		id: `copy-${job.id}`, title: platform.demo ? `演示：${title}（未写磁盘）` : title, text, type, dismissible: !running, copyable: !running,
		buttons: running ? [{ label: '取消复制', action: async () => {
			try { await platform.files.cancel(job.id) }
			catch (error) { vault.error = String(error) }
		}, keepOpen: true }] : [],
	}, running)
}
watch(() => vault.jobs, jobs => {
	const ids = new Set(jobs.map(job => job.id))
	for (const [id, state] of phases) {
		if (!ids.has(id)) {
			if (state === 'running') props.manager.removeNotification(`copy-${id}`)
			phases.delete(id)
		}
	}
	for (const job of jobs) {
		const previous = phases.get(job.id)
		phases.set(job.id, job.state)
		// 首次查询不把数据库/后台保留的旧结束任务当作本次刚完成。
		if (!initialized && job.state !== 'running') continue
		if (job.state === 'running' || previous !== job.state) jobNotice(job)
	}
	initialized = true
}, { deep: true })
watch(() => vault.error, error => {
	if (error) props.manager.notify({ title: '操作未完成', text: error, type: 'error' })
})
watch(() => vault.config.app.notificationSeconds, () => props.manager.retime())
onUnmounted(() => props.manager.clearAllNotifications())
</script>
<template>
	<NotificationPanel class="vault-notifications" />
</template>
<style scoped>
.vault-notifications { bottom: 5rem; width: min(460px, calc(100vw - 3rem)); max-height: calc(100dvh - 9rem); overflow-y: auto; overscroll-behavior: contain; }
</style>
