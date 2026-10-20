<script lang="ts">
	import { ClientReportsModel } from '$lib/components/settings/client-reports-draft.svelte';
	import ClientReportScheduleLine from '$lib/components/settings/client-report-schedule-line.svelte';
	import ClientReportsPreview from '$lib/components/settings/client-reports-preview.svelte';
	import ClientReportsSettingsForm from '$lib/components/settings/client-reports-settings-form.svelte';
	import SendingOffLine from '$lib/components/settings/sending-off-line.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { ListDraft } from '$lib/components/settings/list-draft.svelte';
	import { RECIPIENT_LIST } from '$lib/components/settings/lists';
	import { settings } from '$lib/state/settings.svelte';

	const recipients = new ListDraft(RECIPIENT_LIST);
	const m = new ClientReportsModel({ source: { shared: settings }, recipientCount: () => recipients.count });
</script>

<SettingsPage
	page="client-reports"
	hint="A monthly email to each organization about its domains."
	docs="client-reports#when-it-is-sent"
	tag="Experimental"
	feature={m.feature}
>
	{#snippet below()}
		<div class="space-y-1">
			<ClientReportScheduleLine />
			<SendingOffLine />
		</div>
	{/snippet}
	<ClientReportsSettingsForm model={m} {recipients} />
	{#snippet ungated()}
		<ClientReportsPreview model={m} />
	{/snippet}
</SettingsPage>
