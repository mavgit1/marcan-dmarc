<script lang="ts">
	import './layout.css';
	import { afterNavigate, onNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { ModeWatcher } from 'mode-watcher';
	import AppHeader from '$lib/components/app-header.svelte';
	import AppSidebar from '$lib/components/app-sidebar.svelte';
	import * as Sidebar from '$lib/components/ui/sidebar/index.js';
	import favicon from '$lib/assets/favicon.svg';
	import faviconLight from '$lib/assets/favicon-light.svg';
	import faviconDark from '$lib/assets/favicon-dark.svg';
	import { adoptScopeUrl, syncScopeUrl } from '$lib/state/filters.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';

	let { children } = $props();

	// The scope filter lives in the URL: read before the first page renders and on back /
	// forward; carried over by in-app links (which do not name it); written after each navigation.
	adoptScopeUrl(page.url, 'reset');
	onNavigate(({ to, type }) => {
		if (to) adoptScopeUrl(to.url, type === 'popstate' ? 'reset' : 'carry');
	});
	afterNavigate(() => syncScopeUrl());

	// The 5-minute auto-refresh of every page: a timer of its own, so it keeps going when the
	// health poll fails; paused while an import runs (its progress and end refresh instead).
	$effect(() => refresh.startAutoRefresh(() => systemHealth.current?.imap_progress != null));
</script>

<svelte:head>
	<link rel="icon" type="image/svg+xml" href={favicon} />
	<link rel="icon" type="image/svg+xml" href={faviconLight} media="(prefers-color-scheme: light)" />
	<link rel="icon" type="image/svg+xml" href={faviconDark} media="(prefers-color-scheme: dark)" />
	<title>Marcan</title>
</svelte:head>

<ModeWatcher defaultMode="system" track={true} disableHeadScriptInjection={true} />

<Sidebar.Provider>
	<AppSidebar />
	<Sidebar.Inset class="min-w-0">
		<AppHeader />
		<div class="flex-1 px-4 py-5 md:px-6">
			{@render children()}
		</div>
	</Sidebar.Inset>
</Sidebar.Provider>
