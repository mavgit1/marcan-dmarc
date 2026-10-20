<script lang="ts">
	import { beforeNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import PageHeader from '$lib/components/page-header.svelte';
	import { focusUrlHash } from '$lib/state/anchor-focus.svelte';
	import { holdForUnsaved } from '$lib/components/settings/settings-form.svelte';
	import SettingsNav from '$lib/components/settings/settings-nav.svelte';
	import { pageFromPath } from '$lib/settings-nav';
	import type { Snippet } from 'svelte';

	/**
	 * Settings: the pages by group in a column at the left from `lg` up, the page beside it.
	 * Narrower, `/settings` is the list of pages and each page has its own back link.
	 */
	let { children }: { children: Snippet } = $props();

	const active = $derived(pageFromPath(page.url.pathname));

	// A hash names a section (or a row in one) to show: scrolled to once it is on the page.
	$effect(() => focusUrlHash(page.url.hash));

	// Unsaved edits hold a move to another page; the save bar shows itself instead.
	beforeNavigate(holdForUnsaved);
</script>

<PageHeader title="Settings" class={active ? 'max-lg:hidden' : undefined} />

<div class="lg:grid lg:grid-cols-[13rem_minmax(0,1fr)] lg:items-start lg:gap-8">
	<aside class="max-lg:hidden lg:sticky lg:top-below-header">
		<SettingsNav variant="column" {active} />
	</aside>
	<div class="min-w-0">
		{@render children()}
	</div>
</div>
