import { AbstractWebNotificationManager, type NotificationPanelLocation, type WebNotification } from '@modrinth/ui'
import { ref } from 'vue'

// 使用 Celestial 原 manager 的去重、悬停暂停、手动关闭和计时机制。
export class AppNotificationManager extends AbstractWebNotificationManager {
	private readonly state = ref<WebNotification[]>([])
	private readonly location = ref<NotificationPanelLocation>('right')
	constructor(private readonly seconds: () => number) { super() }
	getNotificationLocation() { return this.location.value }
	setNotificationLocation(location: NotificationPanelLocation) { this.location.value = location }
	getNotifications() { return this.state.value }
	protected addNotificationToStorage(notification: WebNotification) {
		if (this.state.value.length >= 6) {
			const oldest = this.state.value.find(item => item.dismissible !== false)
			if (oldest) this.removeNotification(oldest.id)
		}
		this.state.value.push(notification)
	}
	protected removeNotificationFromStorage(id: string | number) {
		const index = this.state.value.findIndex(item => item.id === id)
		if (index !== -1) this.state.value.splice(index, 1)
	}
	protected removeNotificationFromStorageByIndex(index: number) { this.state.value.splice(index, 1) }
	protected clearAllNotificationsFromStorage() { this.state.value.splice(0) }
	private autoCloseMs() {
		const seconds = this.seconds()
		return seconds === 0 ? null : (Number.isFinite(seconds) ? Math.min(120, Math.max(1, seconds)) : 6) * 1000
	}
	notify(notification: Partial<WebNotification>) {
		return this.addNotification({ autoCloseMs: this.autoCloseMs(), ...notification })
	}
	updateJob(notification: WebNotification, running: boolean) {
		const existing = this.state.value.find(item => item.id === notification.id)
		const updated = { ...notification, autoCloseMs: running ? null : this.autoCloseMs() }
		if (existing) {
			Object.assign(existing, updated)
			if (!running) this.setNotificationTimer(existing)
		} else this.addNotification(updated)
	}
	retime() {
		for (const item of this.state.value) {
			if (item.dismissible === false) continue
			item.autoCloseMs = this.autoCloseMs(); this.setNotificationTimer(item)
		}
	}
}
