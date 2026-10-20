<script lang="ts">
	import { AlertsModel } from '$lib/components/settings/alerts-draft.svelte';
	import AlertsSettingsForm from '$lib/components/settings/alerts-settings-form.svelte';
	import SendingOffLine from '$lib/components/settings/sending-off-line.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { settings } from '$lib/state/settings.svelte';

	const m = new AlertsModel({ source: { shared: settings } });
</script>

<SettingsPage
	page="alerts"
	hint="Emails about changes to your domains and a regular digest; the Alerts page lists every change either way."
	docs="alerts#alert-emails"
	tag="Experimental"
	feature={m.feature}
>
	{#snippet below()}
		<SendingOffLine />
	{/snippet}
	<AlertsSettingsForm model={m} />
</SettingsPage>
