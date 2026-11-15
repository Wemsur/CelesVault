import { createRouter, createWebHashHistory } from 'vue-router'

const routes = [
	{ path: '/', name: 'home', component: () => import('@/features/home/HomePage.vue') },
	{ path: '/library', name: 'library', component: () => import('@/features/library/LibraryPage.vue') },
]
if (import.meta.env.DEV) routes.push({ path: '/__components', name: 'components', component: () => import('@/features/dev/ComponentShowcase.vue') })

export const router = createRouter({ history: createWebHashHistory(), routes })
