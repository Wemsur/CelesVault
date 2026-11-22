import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import svgLoader from 'vite-svg-loader'

const path = (value: string) => fileURLToPath(new URL(value, import.meta.url))

export default defineConfig({
	plugins: [vue(), svgLoader({ svgo: false })],
	resolve: {
		alias: [
			{ find: '@', replacement: path('./src') },
			{ find: /^@modrinth\/ui$/, replacement: path('./src/vendor/celestial/ui/index.ts') },
			{ find: /^@modrinth\/assets$/, replacement: path('./src/vendor/celestial/assets/index.ts') },
			{ find: '@modrinth/assets/styles', replacement: path('./src/vendor/celestial/styles') },
			{ find: '#ui', replacement: path('./src/vendor/celestial/ui') },
		],
	},
	server: {
		// CLI selects LAN (web:dev) or loopback (web:dev:local / Tauri).
		host: '127.0.0.1',
		fs: { strict: true, allow: [path('./')], deny: ['.env', '.env.*', '*.{crt,pem}', '**/.git/**', '**/data/**', '**/target/**', '**/celesvault-location.json'] },
		port: 8006,
		strictPort: true,
		watch: {
			ignored: [
				'**/target/**',
				'**/src-tauri/**',
				'**/crates/**',
				'**/tools/wps-probe/**',
				'**/data/**',
				'**/dist/**',
				'**/web-dist/**',
				'**/Cargo.lock',
				'**/celesvault-location.json',
			],
		},
	},
	build: { outDir: 'web-dist', emptyOutDir: true },
	clearScreen: false,
})
