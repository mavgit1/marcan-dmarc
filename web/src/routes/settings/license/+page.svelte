<script lang="ts">
	import { LicenseModel } from '$lib/components/settings/license-draft.svelte';
	import LicenseSettingsForm from '$lib/components/settings/license-settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import { UpdatesModel } from '$lib/components/settings/updates-draft.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import AboutSection from './about-section.svelte';
	import ClientEmailsSection from './client-emails-section.svelte';

	const m = new LicenseModel({ source: { shared: settings } });
	const updates = new UpdatesModel({ source: { shared: settings } });
</script>

<!-- One form: the key, and in About the version check's switch, saved by the one save bar. -->
<SettingsPage
	page="license"
	hint="Your operator key and its subscription."
	docs="license#the-operator-key-and-the-notice"
>
	<LicenseSettingsForm model={m} />
	<div class="mt-8 space-y-4">
		<ClientEmailsSection />
		<AboutSection model={updates} />
	</div>
</SettingsPage>
