import type { Config } from 'tailwindcss'
import preset from './src/vendor/celestial/styles/tailwind-preset'

export default {
	presets: [preset],
	content: ['./index.html', './src/**/*.{vue,ts}'],
} satisfies Config
