<script lang="ts">
	import { resolve } from '$app/paths';
	import StatusBadge from '#lib/components/status-badge.svelte';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { DOCS_URL } from '#lib/nav.ts';
	import { settingsPage, type SettingsPageId } from '#lib/settings-nav.ts';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';

	/**
	 * The top of a Settings page: a back link to the list (narrow screens only, where the list is
	 * its own page), the page's title with one sentence of what it configures under it, and at
	 * the title's right the quiet outline "Docs" button that `/alerts` and the DNS inspector use
	 * (it wraps under the title at 375 px, never onto a line of the sentence). `tag` is a label in
	 * the warn tone after the title ("Experimental"). A page below another (an account under
	 * Accounts) passes its own `title` and `back` link.
	 */
	let {
		page,
		title,
		back,
		hint,
		docs,
		tag
	}: {
		page: SettingsPageId;
		/** In place of the page's title (an account's name). */
		title?: string;
		/** Where the back link goes, in place of the Settings list (shown at every width). */
		back?: { href: string; label: string };
		/** One sentence: what the page configures. */
		hint?: string;
		/** The docs page and anchor, such as `accounts#sending`. */
		docs?: string;
		tag?: string;
	} = $props();
</script>

<header class="mb-4">
	<a
		href={back?.href ?? resolve('/settings')}
		class={[
			'text-muted-foreground hover:text-foreground -ml-1 mb-2 inline-flex items-center gap-0.5 text-sm',
			!back && 'lg:hidden'
		]}
	>
		<ChevronLeftIcon class="size-4" />
		{back?.label ?? 'Settings'}
	</a>
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div class="min-w-0 space-y-1">
			<h2 class="flex flex-wrap items-center gap-2 text-lg font-semibold tracking-tight break-words">
				{title ?? settingsPage(page)?.title}
				{#if tag}<StatusBadge tone="warn" class="text-[10px] font-normal tracking-normal">{tag}</StatusBadge>{/if}
			</h2>
			{#if hint}
				<p class="text-muted-foreground max-w-2xl text-sm">{hint}</p>
			{/if}
		</div>
		{#if docs}
			<Button variant="outline" size="sm" href="{DOCS_URL}{docs}" target="_blank" rel="noopener" class="shrink-0">
				<BookOpenIcon />
				Docs
			</Button>
		{/if}
	</div>
</header>
