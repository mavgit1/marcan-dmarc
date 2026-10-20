// Vitest environment for the UI's unit tests: plain node (no DOM), but the test files and
// everything they import go through Vite's *client* environment, so `.svelte.ts` state modules
// (resource, SettingsForm) compile with the client runtime and `$effect.root` / `flushSync`
// work. Imported natively by node, hence plain JS.
/** @type {import('vitest/environments').Environment} */
export default {
	name: 'client-node',
	viteEnvironment: 'client',
	setup() {
		return { teardown() {} };
	}
};
