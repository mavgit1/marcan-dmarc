<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { saveOnboarding } from '$lib/api';
	import type { OnboardingState } from '$lib/types/OnboardingState';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import SetupStepper from '$lib/components/setup/setup-stepper.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { settings } from '$lib/state/settings.svelte';

	/**
	 * The setup. The dashboard opens it until it is finished or skipped (the onboarding settings
	 * group); Settings → Data runs it again.
	 */
	const finish = new Action();
	const skip = new Action();
	/** Skip only while the setup is open: once finished or skipped, there is nothing to skip. */
	const open = $derived(settings.current?.onboarding.state === null);

	/** Store how the setup ended first, so the dashboard agrees, then open it. */
	function end(action: Action, state: OnboardingState) {
		action.run(async () => {
			await settings.save(saveOnboarding(state));
			refresh.refresh();
			await goto(resolve('/'));
		});
	}
</script>

<div class="space-y-3">
	<SetupStepper finishing={finish.busy} onFinish={() => end(finish, 'completed')}>
		{#snippet header()}
			{#if open}
				<BusyButton
					variant="ghost"
					size="sm"
					busy={skip.busy}
					busyLabel="Skipping…"
					onclick={() => end(skip, 'skipped')}
				>
					Skip setup
				</BusyButton>
			{/if}
		{/snippet}
	</SetupStepper>
	<div class="mx-auto max-w-3xl">
		<FeedbackAlert feedback={finish.feedback ?? skip.feedback} failTitle="Could not save" />
	</div>
</div>
