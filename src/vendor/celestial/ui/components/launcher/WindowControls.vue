<!-- CelesVault platform adaptation, 2026-11-14; preserves original template, Teleport and scoped styles. -->
<template>
	<Teleport to="body">
		<section
			v-if="showControls"
			ref="controlsEl"
			class="pointer-events-auto fixed top-0 right-0 z-[10001] flex h-12 items-center gap-2 rounded-bl-2xl px-1.5"
			data-tauri-drag-region-exclude
		>
			<IconButton
				type="quiet"
				label="最小化窗口"
				class="relative expanded-button"
				@click="performWindowAction(() => platform.window.minimize())"
			>
				<MinimizeIcon />
			</IconButton>
			<IconButton
				type="quiet"
				label="最大化或还原窗口"
				class="relative expanded-button"
				@click="handleToggleMaximize"
			>
				<RestoreIcon v-if="isMaximized" />
				<MaximizeIcon v-else />
			</IconButton>
			<IconButton
				type="quiet"
				label="隐藏到托盘"
				class="relative expanded-button close-button"
				@click="handleClose"
			>
				<XIcon />
			</IconButton>
		</section>
	</Teleport>
</template>

<script setup lang="ts">
import { MaximizeIcon, MinimizeIcon, RestoreIcon, XIcon } from '@modrinth/assets'
import { onMounted, onUnmounted, ref, watch } from 'vue'

import { usePlatform } from '@/platform'
import IconButton from '../base/buttons/IconButton.vue'

withDefaults(defineProps<{ showControls?: boolean }>(), { showControls: true })
const emit = defineEmits<{
	'width-change': [width: number]
	'window-error': [error: unknown]
}>()
const platform = usePlatform()
const isMaximized = ref(false)
const controlsEl = ref<HTMLElement | null>(null)
let resizeObserver: ResizeObserver | undefined
let unlisten: (() => void) | undefined
let disposed = false

function setWindowControlsWidth(width: number) {
	document.documentElement.style.setProperty('--window-controls-width', `${width}px`)
	emit('width-change', width)
}

watch(controlsEl, (el) => {
	resizeObserver?.disconnect()
	resizeObserver = undefined
	if (!el) {
		setWindowControlsWidth(0)
		return
	}
	resizeObserver = new ResizeObserver(() => {
		setWindowControlsWidth(el.getBoundingClientRect().width)
	})
	resizeObserver.observe(el)
	setWindowControlsWidth(el.getBoundingClientRect().width)
})

async function performWindowAction(action: () => unknown) {
	try {
		await action()
	} catch (error) {
		emit('window-error', error)
	}
}

async function handleToggleMaximize() {
	await performWindowAction(async () => {
		await platform.window.toggleMaximize()
		const state = await platform.window.getState()
		if (!disposed) isMaximized.value = state.maximized
	})
}

async function handleClose() {
	await performWindowAction(() => platform.window.hide())
}

onMounted(async () => {
	try {
		unlisten = await platform.window.onStateChanged((state) => {
			if (!disposed) isMaximized.value = state.maximized
		})
		if (disposed) {
			unlisten()
			return
		}
		const state = await platform.window.getState()
		if (!disposed) isMaximized.value = state.maximized
	} catch (error) {
		emit('window-error', error)
	}
})

onUnmounted(() => {
	disposed = true
	unlisten?.()
	resizeObserver?.disconnect()
	document.documentElement.style.removeProperty('--window-controls-width')
	emit('width-change', 0)
})
</script>
<style scoped>
.expanded-button::before {
	inset: -9px -6px;
	content: '';
	position: absolute;
}

.expanded-button.close-button::before {
	inset: -9px -9px -9px -6px;
}
</style>
