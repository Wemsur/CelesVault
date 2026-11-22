import { createApp } from 'vue'
import { createPinia } from 'pinia'
import FloatingVue from 'floating-vue'
import 'floating-vue/dist/style.css'
import 'overlayscrollbars/styles/overlayscrollbars.css'
import '@/styles/app.scss'
import App from './App.vue'
import { router } from './router'
import { createPlatform, platformKey } from './platform'

async function mount() {
	const teleports = document.createElement('div')
	teleports.id = 'teleports'
	document.body.append(teleports)
	const platform = await createPlatform()
	document.documentElement.classList.toggle('vault-demo-mode', platform.demo === true)
	const app = createApp(App)
	app.use(createPinia()).use(router).use(FloatingVue)
	app.provide(platformKey, platform)
	app.mount('#app')
}

mount().catch(error => {
	const root = document.getElementById('app')
	if (root) {
		root.style.padding = '2rem'
		root.textContent = `CelesVault 无法启动：${error instanceof Error ? error.message : String(error)}`
	}
	console.error(error)
})
