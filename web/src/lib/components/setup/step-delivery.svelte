<script lang="ts">
	import ImapDeliveryForm from '$lib/components/settings/imap-delivery-form.svelte';
	import { REPORT_DELIVERY_INTRO } from '$lib/reporting-modes';
	import type { SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/** How the domains' reports reach the mailbox; Continue saves the choice. */
	let { data, onBack, onNext }: { data: SetupData; onBack: () => void; onNext: () => void } = $props();
</script>

<div class="space-y-4">
	{#if !data.settings.mailbox.configured}
		<p class="text-muted-foreground text-xs leading-relaxed">{REPORT_DELIVERY_INTRO}</p>
		<p class="text-muted-foreground text-xs">Connect the mailbox first; then choose how reports reach it.</p>
		<StepNav {onBack} onContinue={onNext} />
	{:else}
		<p class="text-muted-foreground text-xs leading-relaxed">{REPORT_DELIVERY_INTRO}</p>
		<ImapDeliveryForm settings={data.settings} idPrefix="setup-delivery" onSaved={onNext}>
			{#snippet actions(busy)}
				<StepNav {onBack} submit {busy} label="Save and continue" />
			{/snippet}
		</ImapDeliveryForm>
	{/if}
</div>
