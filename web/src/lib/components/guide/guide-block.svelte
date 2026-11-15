<script lang="ts">
	import type { Snippet } from 'svelte';

	/**
	 * One block of Tools → Guide: its title with the docs links beside it, what it explains with
	 * this installation's values, and where that part stands (`status`, optional).
	 */
	let {
		id,
		title,
		links,
		body,
		status
	}: { id: string; title: string; links: Snippet; body: Snippet; status?: Snippet } = $props();
</script>

<section aria-labelledby="guide-{id}" class="space-y-1.5 py-4 first:pt-0">
	<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
		<h2 id="guide-{id}" class="text-sm font-medium">{title}</h2>
		<div class="flex items-center gap-3">{@render links()}</div>
	</div>
	<div class="text-muted-foreground leading-relaxed">{@render body()}</div>
	{#if status}
		<div class="text-xs leading-relaxed">{@render status()}</div>
	{/if}
</section>
