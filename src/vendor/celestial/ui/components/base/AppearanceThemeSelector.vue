<!-- CelesVault adaptation 2026-11-21: Chinese labels, three themes, no i18n. -->
<script setup lang="ts" generic="T extends string">
import { MoonIcon, RadioButtonCheckedIcon, RadioButtonIcon, SunIcon } from '@modrinth/assets'
import { computed } from 'vue'


const { ariaLabel, modelValue, themeOptions, systemThemeColor, preferredDarkTheme } = defineProps<{
	ariaLabel: string
	modelValue: T
	themeOptions: readonly T[]
	systemThemeColor: T
	preferredDarkTheme: T
}>()

const visibleThemeOptions = computed(() =>
	themeOptions.filter((option) => !['light', 'dark'].includes(option)),
)

const emit = defineEmits<{
	'update:modelValue': [theme: T]
}>()

const themeLabels: Record<string,string> = { customdark:'深色', customlight:'浅色', system:'跟随系统' }
function formatTheme(theme: T): string { return themeLabels[theme] ?? theme }
function formatTooltip(key: string): string { return key === 'preferredLight' ? '浅色主题' : '深色主题' }

function getPreviewClass(option: T): string {
	const base = option === 'system' ? systemThemeColor : option
	return base.endsWith('-mode') ? base : `${base}-mode`
}
</script>

<template>
	<div class="theme-options" role="group" :aria-label="ariaLabel">
		<button
			v-for="option in visibleThemeOptions"
			:key="option"
			type="button"
			class="preview-radio button-base"
			:class="{ selected: modelValue === option }"
			:aria-pressed="modelValue === option"
			@click="emit('update:modelValue', option)"
		>
			<div class="preview" :class="getPreviewClass(option)" aria-hidden="true">
				<div class="example-card rounded-lg border border-solid border-surface-4 bg-surface-3">
					<div class="example-icon"></div>
					<div class="example-text-1"></div>
					<div class="example-text-2"></div>
				</div>
			</div>
			<div class="label">
				<RadioButtonCheckedIcon
					v-if="modelValue === option"
					class="radio shrink-0"
					aria-hidden="true"
				/>
				<RadioButtonIcon v-else class="radio shrink-0" aria-hidden="true" />
				{{ formatTheme(option) }}
				<SunIcon
					v-if="'customlight' === option"
					v-tooltip="formatTooltip('preferredLight')"
					class="theme-icon shrink-0"
					aria-hidden="true"
				/>
				<MoonIcon
					v-else-if="'customdark' === option"
					v-tooltip="formatTooltip('preferredDark')"
					class="theme-icon shrink-0"
					aria-hidden="true"
				/>
                <span
                    v-if="'customlight' === option"
                    class="rounded-full px-1.5 py-0.5 text-xs ml-auto mr-0 font-bold bg-brand-highlight text-brand-green"
                >
						自由配色
					</span>
                <span
                    v-else-if="'customdark' === option"
                    class="rounded-full px-1.5 py-0.5 text-xs ml-auto mr-0 font-bold bg-brand-highlight text-brand-green"
                >
						自由配色
					</span>
                <span
                    v-else-if="'system' === option"
                    class="rounded-full px-1.5 py-0.5 text-xs ml-auto mr-0 font-bold bg-brand-highlight text-brand-green"
                >
                        自由配色
					</span>
			</div>
		</button>
	</div>
</template>

<style scoped lang="scss">
.theme-options {
	display: grid;
	grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
	gap: var(--gap-lg);

	.preview {
        &.customdark-mode {
            /* Global customdark tokens. */
        }
        &.customlight-mode {
            /* Global customlight tokens. */
        }
	}

	.preview .example-card {
		margin: 0;
		padding: 1rem;
		display: grid;
		grid-template: 'icon text1' 'icon text2';
		grid-template-columns: auto 1fr;
		gap: 0.5rem;
		outline: 2px solid transparent;

		.example-icon {
			grid-area: icon;
			width: 2rem;
			height: 2rem;
			background-color: var(--color-button-bg);
			border-radius: var(--radius-sm);
			outline: 2px solid transparent;
		}

		.example-text-1,
		.example-text-2 {
			height: 0.5rem;
			border-radius: var(--radius-sm);
			outline: 2px solid transparent;
		}

		.example-text-1 {
			grid-area: text1;
			width: 100%;
			background-color: var(--color-base);
		}

		.example-text-2 {
			grid-area: text2;
			width: 60%;
			background-color: var(--color-secondary);
		}
	}
}
</style>
