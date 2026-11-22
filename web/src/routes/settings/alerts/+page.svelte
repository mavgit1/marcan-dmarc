<script lang="ts">
	import { AlertsModel } from '#lib/components/settings/alerts-draft.svelte.ts';
	import AlertsSettingsForm from '#lib/components/settings/alerts-settings-form.svelte';
	import SendingOffLine from '#lib/components/settings/sending-off-line.svelte';
	import SettingsPage from '#lib/components/settings/settings-page.svelte';
	import { settings } from '#lib/state/settings.svelte.ts';

	const m = new AlertsModel({ source: { shared: settings } });
</script>

<SettingsPage
	page="alerts"
	hint="Emails about changes to your domains, and a regular digest."
	docs="alerts#alert-emails"
	tag="Experimental"
	feature={m.feature}
>
	{#snippet below()}
		<SendingOffLine />
	{/snippet}
	<AlertsSettingsForm model={m} />
</SettingsPage>
