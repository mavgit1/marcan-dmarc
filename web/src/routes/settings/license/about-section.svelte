<script lang="ts">
	import { DOCS_URL } from '$lib/nav';
	import { meta } from '$lib/state/meta.svelte';

	/**
	 * The foot of the License page, neutral text from `meta`: the version, the license (free
	 * software, no warranty, AGPLv3 plus one duty) and the source code. Not the page's headline:
	 * that is the license state box.
	 */
	const version = $derived(meta.current?.version ?? null);
	/** The server's one source URL (also in the notice and the emails). */
	const sourceUrl = $derived(meta.current?.source_url ?? null);
	$effect(() => {
		meta.load().catch(() => {});
	});
	const link = 'text-foreground underline underline-offset-2';
</script>

<section aria-labelledby="license-about" class="max-w-prose space-y-2">
	<h2 id="license-about" class="text-sm font-medium">About</h2>
	<p class="text-muted-foreground text-sm">
		Marcan <span class="tabular-nums">{version ?? '—'}</span>. Free software, no warranty: Beneficiary Copyleft License
		1.0, which is AGPLv3 plus one duty. Anyone you let use it may ask for the source code. The client emails tell your
		clients that it is free software (the free-software notice); an operator license replaces that notice with your
		name.
	</p>
	<p class="text-muted-foreground text-sm">
		Source code:
		{#if sourceUrl}
			<a href={sourceUrl} target="_blank" rel="noopener" class="{link} break-all">{sourceUrl}</a>
		{:else}
			—
		{/if}
		· <a href="{DOCS_URL}license" target="_blank" rel="noopener" class={link}>License &amp; pricing</a>
	</p>
</section>
