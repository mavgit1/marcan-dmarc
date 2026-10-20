<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { saveOnboarding } from '$lib/api';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import ReimportButton from '$lib/components/settings/reimport-button.svelte';
	import SettingRow from '$lib/components/settings/setting-row.svelte';
	import SettingsSection from '$lib/components/settings/settings-section.svelte';
	import { settings } from '$lib/state/settings.svelte';

	/**
	 * The Data page's Actions, outside its form: each acts at once with its own feedback
	 * (re-import, the setup). Nothing here is a setting.
	 */
	const rerunSetup = new Action();

	function onRerunSetup() {
		rerunSetup.run(async () => {
			await settings.save(saveOnboarding(null));
			await goto(resolve('/setup'));
		});
	}
</script>

<SettingsSection title="Actions">
	<SettingRow title="Re-import" hint="Reads every message again and adds missing reports.">
		<ReimportButton />
	</SettingRow>
	<SettingRow title="Setup" hint="Opens the setup steps again.">
		<div class="space-y-2">
			<BusyButton
				type="button"
				variant="outline"
				size="sm"
				busy={rerunSetup.busy}
				busyLabel="Opening…"
				onclick={onRerunSetup}
			>
				Run setup again
			</BusyButton>
			<FeedbackAlert feedback={rerunSetup.feedback} failTitle="Could not reopen setup" />
		</div>
	</SettingRow>
</SettingsSection>
