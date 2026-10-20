<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import SettingsNav from '$lib/components/settings/settings-nav.svelte';
	import { MediaQuery } from 'svelte/reactivity';

	/**
	 * `/settings`: on a narrow screen the list of pages; from `lg` up, where the list is the
	 * column beside every page, the first page.
	 */
	const wide = new MediaQuery('min-width: 64rem');
	$effect(() => {
		if (wide.current) void goto(resolve('/settings/mailbox'), { replaceState: true });
	});
</script>

<div class="lg:hidden">
	<SettingsNav variant="list" />
</div>
