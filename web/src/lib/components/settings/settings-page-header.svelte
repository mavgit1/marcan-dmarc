<script lang="ts">
	import { resolve } from '$app/paths';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { DOCS_URL } from '$lib/nav';
	import { settingsPage, type SettingsPageId } from '$lib/settings-nav';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';

	/**
	 * The top of a Settings page: a back link to the list (narrow screens only, where the list is
	 * its own page), the page's title, one sentence of what it configures and one Docs link.
	 * `tag` is a label in the warn tone after the title ("Experimental").
	 */
	let {
		page,
		hint,
		docs,
		tag
	}: {
		page: SettingsPageId;
		/** One sentence: what the page configures. */
		hint?: string;
		/** The docs page and anchor, such as `mailbox#sending`. */
		docs?: string;
		tag?: string;
	} = $props();
</script>

<header class="mb-4 space-y-1">
	<a
		href={resolve('/settings')}
		class="text-muted-foreground hover:text-foreground -ml-1 mb-2 inline-flex items-center gap-0.5 text-sm lg:hidden"
	>
		<ChevronLeftIcon class="size-4" />
		Settings
	</a>
	<h2 class="flex flex-wrap items-center gap-2 text-lg font-semibold tracking-tight">
		{settingsPage(page)?.title}
		{#if tag}<StatusBadge tone="warn" class="text-[10px] font-normal tracking-normal">{tag}</StatusBadge>{/if}
	</h2>
	{#if hint || docs}
		<p class="text-muted-foreground max-w-2xl text-sm">
			{hint}
			{#if docs}
				<a href="{DOCS_URL}{docs}" target="_blank" rel="noopener" class="text-foreground underline underline-offset-2"
					>Docs</a
				>
			{/if}
		</p>
	{/if}
</header>
