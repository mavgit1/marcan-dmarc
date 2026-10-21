import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

/**
 * Under Vitest the bare `svelte` import must be its client entry: node would pick the server
 * build (whose `untrack` / `flushSync` are stubs) while compiled runes use the client runtime,
 * and two runtimes make effects loop. The real path, so node dedupes it with the internals.
 */
const svelteClient = decodeURIComponent(new URL(import.meta.resolve('svelte/internal/client')).pathname).replace(
	/internal\/client\/index\.js$/,
	'index-client.js'
);

export default defineConfig(({ mode }) => ({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
			},

			// Static SPA for rust-embed: all non-API routes fall back to index.html.
			adapter: adapter({
				fallback: 'index.html'
			}),
			paths: {
				relative: false
			}
		})
	],
	resolve: mode === 'test' ? { alias: [{ find: /^svelte$/, replacement: svelteClient }] } : undefined,
	test: {
		// Unit tests of the UI's pure logic (wording, formats, DMARC record edits) and of its
		// state modules. No DOM: node, with Vite's client environment so `.svelte.ts` modules
		// (resource, SettingsForm) compile with the client runtime (`vitest.environment.js`).
		// A component's tests (`*.svelte.test.ts` beside it, `$lib/test/component-test.svelte`)
		// ask for happy-dom in their first line (`// @vitest-environment happy-dom`).
		include: ['src/**/*.test.ts'],
		environment: './vitest.environment.js'
	},
	server: {
		host: '127.0.0.1',
		port: 5173,
		// Dev: proxy /api to dmarc-server (cargo run -p dmarc-server).
		proxy: {
			'/api': {
				target: 'http://127.0.0.1:8080',
				changeOrigin: true
			}
		}
	}
}));
