<script lang="ts">
	import { LicenseModel } from '$lib/components/settings/license-draft.svelte';
	import LicenseSettingsForm from '$lib/components/settings/license-settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import AboutSection from './about-section.svelte';
	import ClientEmailsSection from './client-emails-section.svelte';

	const m = new LicenseModel({ source: { shared: settings } });
</script>

<SettingsPage
	page="license"
	hint="Your operator key and its subscription."
	docs="license#the-operator-key-and-the-notice"
>
	<LicenseSettingsForm model={m} />
	{#snippet after()}
		<ClientEmailsSection />
		<AboutSection />
	{/snippet}
</SettingsPage>
