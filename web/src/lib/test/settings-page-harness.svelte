<script lang="ts" generics="P extends Record<string, unknown>">
	import type { Component } from 'svelte';
	import type { SettingsPageId } from '$lib/settings-nav';
	import type { Feature } from '$lib/components/settings/settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';

	/**
	 * A Settings page around one form, for the component tests: the page form, its save bar and,
	 * with `feature`, the page's switch, as a route's `+page.svelte` puts them together.
	 */
	let {
		page = 'sending',
		feature,
		form: Form,
		props
	}: {
		page?: SettingsPageId;
		/** The page's switch (`model.feature`), read on each render. */
		feature?: () => Feature | null;
		form: Component<P>;
		props: P;
	} = $props();
</script>

<SettingsPage {page} hint="A page under test." docs="index" feature={feature?.()}>
	<Form {...props} />
</SettingsPage>
