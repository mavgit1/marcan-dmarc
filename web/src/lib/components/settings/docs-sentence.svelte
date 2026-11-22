<script lang="ts">
	import { DOCS_URL } from '#lib/nav.ts';

	/**
	 * A server sentence that ends with a "(docs)" link to the docs section showing the fix (`docs`,
	 * relative to the docs' root, as the server sends it): "… test again later (docs)." Without
	 * `docs`, the sentence alone.
	 */
	let { message, docs = null }: { message: string; docs?: string | null } = $props();

	const stop = $derived(docs && message.endsWith('.') ? '.' : '');
</script>

{#if docs}{stop ? message.slice(0, -1) : message} (<a
		href="{DOCS_URL}{docs}"
		target="_blank"
		rel="noreferrer"
		class="underline underline-offset-2">docs</a
	>){stop}{:else}{message}{/if}
