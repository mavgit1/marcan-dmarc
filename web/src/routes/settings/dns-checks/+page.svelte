<script lang="ts">
	import { ListDraft } from '#lib/components/settings/list-draft.svelte.ts';
	import ListEditor from '#lib/components/settings/list-editor.svelte';
	import { DKIM_SELECTOR_LIST } from '#lib/components/settings/lists.ts';
	import { ResolversModel } from '#lib/components/settings/resolvers-draft.svelte.ts';
	import SettingsPage from '#lib/components/settings/settings-page.svelte';
	import SettingsSection from '#lib/components/settings/settings-section.svelte';
	import { settings } from '#lib/state/settings.svelte.ts';
	import ResolversRow from './resolvers-row.svelte';

	const resolvers = new ResolversModel({ source: { shared: settings } });
	const selectors = new ListDraft(DKIM_SELECTOR_LIST);
</script>

<SettingsPage
	page="dns-checks"
	hint="What every DNS check uses: the resolvers, and DKIM selectors to try besides those seen in reports."
	docs="dns-checks#resolvers"
>
	<SettingsSection>
		<ResolversRow model={resolvers} />
		<ListEditor list={selectors} row hint="One per line." anchor="dkim-selectors" />
	</SettingsSection>
</SettingsPage>
