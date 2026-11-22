<script lang="ts">
	import { BrandingModel } from '#lib/components/settings/branding-draft.svelte.ts';
	import BrandingSettingsForm from '#lib/components/settings/branding-settings-form.svelte';
	import SettingsPage from '#lib/components/settings/settings-page.svelte';
	import { settings } from '#lib/state/settings.svelte.ts';

	const m = new BrandingModel({ source: { shared: settings } });
</script>

<SettingsPage page="branding" hint="Your name and logo, shown in emails only." docs="license#branding">
	<BrandingSettingsForm model={m} />
</SettingsPage>
