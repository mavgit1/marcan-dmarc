<script lang="ts">
	import { RetentionModel } from '$lib/components/settings/retention-draft.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import ActionsSection from './actions-section.svelte';
	import RetentionSection from './retention-section.svelte';
	import StoredDataSection from './stored-data-section.svelte';

	const retention = new RetentionModel({ source: { shared: settings } });
</script>

<SettingsPage
	page="data"
	hint="What the database holds, how long reports are kept, and how to rebuild it."
	docs="backup"
>
	<StoredDataSection />
	<RetentionSection model={retention} />
	{#snippet after()}
		<ActionsSection />
	{/snippet}
</SettingsPage>
