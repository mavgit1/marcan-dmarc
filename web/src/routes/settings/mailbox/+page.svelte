<script lang="ts">
	import { mailboxSettingsModel } from '$lib/components/settings/imap-draft.svelte';
	import ImapSettingsForm from '$lib/components/settings/imap-settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';

	const m = mailboxSettingsModel();
</script>

<SettingsPage
	page="mailbox"
	hint="Where the DMARC reports arrive, and how often Marcan looks."
	docs="mailbox#connect-the-mailbox"
>
	<ImapSettingsForm model={m} />
</SettingsPage>
