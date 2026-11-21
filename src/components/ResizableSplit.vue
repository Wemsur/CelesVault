<script setup lang="ts">
import { computed, ref } from 'vue'
const props = withDefaults(defineProps<{ modelValue: number; vertical?: boolean; minSize?: number; single?: boolean }>(), { vertical: false, minSize: 180, single: false })
const emit = defineEmits<{ 'update:modelValue': [value: number]; commit: [value: number] }>()
const root = ref<HTMLElement | null>(null)
const grid = computed(() => ({ [props.vertical ? 'gridTemplateRows' : 'gridTemplateColumns']: props.single ? 'minmax(0,1fr)' : `minmax(0,${props.modelValue}fr) 6px minmax(0,${1 - props.modelValue}fr)` }))
function move(event: PointerEvent) {
	if (!(event.currentTarget as HTMLElement).hasPointerCapture(event.pointerId) || !root.value) return
	const bounds = root.value.getBoundingClientRect()
	const size = props.vertical ? bounds.height : bounds.width
	if (!size) return
	const min = Math.min(.4, Math.max(.15, props.minSize / size))
	const value = (props.vertical ? event.clientY - bounds.top : event.clientX - bounds.left) / size
	emit('update:modelValue', Math.min(1 - min, Math.max(min, value)))
}
function commit(event: PointerEvent) {
	const separator = event.currentTarget as HTMLElement
	if (separator.hasPointerCapture(event.pointerId)) separator.releasePointerCapture(event.pointerId)
	emit('commit', props.modelValue)
}
function keyboard(event: KeyboardEvent) {
	if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return
	event.preventDefault()
	const step = ['ArrowRight', 'ArrowDown'].includes(event.key) ? .025 : -.025
	const value = Math.min(.85, Math.max(.15, props.modelValue + step))
	emit('update:modelValue', value)
	emit('commit', value)
}
</script>
<template>
	<div ref="root" class="vault-split" :class="{ vertical }" :style="grid">
		<div class="vault-split-pane"><slot name="first" /></div>
		<div v-if="!single" class="vault-split-handle" role="separator" tabindex="0" :aria-orientation="vertical ? 'horizontal' : 'vertical'" :aria-valuenow="Math.round(modelValue * 100)" aria-valuemin="15" aria-valuemax="85" aria-label="调整分屏比例" @pointerdown="($event.currentTarget as HTMLElement).setPointerCapture($event.pointerId)" @pointermove="move" @pointerup="commit" @pointercancel="commit" @keydown="keyboard" />
		<div v-if="!single" class="vault-split-pane"><slot name="second" /></div>
	</div>
</template>
<style scoped>
.vault-split { display: grid; height: 100%; min-height: 0; min-width: 0; }
.vault-split-pane { min-height: 0; min-width: 0; overflow: hidden; }
.vault-split-handle { cursor: col-resize; background: var(--surface-1); position: relative; touch-action: none; }
.vault-split-handle::after { content: ''; position: absolute; inset: 0 2px; background: var(--surface-5); }
.vertical > .vault-split-handle { cursor: row-resize; }
.vertical > .vault-split-handle::after { inset: 2px 0; }
.vault-split-handle:hover, .vault-split-handle:focus-visible { background: var(--color-brand-highlight); }
</style>
