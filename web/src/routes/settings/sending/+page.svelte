<script lang="ts">
	import { SendingModel } from '$lib/components/settings/sending-draft.svelte';
	import SendingSettingsForm from '$lib/components/settings/sending-settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { settings } from '$lib/state/settings.svelte';

	const m = new SendingModel({ source: { shared: settings } });
</script>

<SettingsPage
	page="sending"
	hint="The SMTP account alerts and client reports are sent through."
	docs="mailbox#sending"
	feature={m.feature}
>
	<SendingSettingsForm model={m} />
</SettingsPage>
