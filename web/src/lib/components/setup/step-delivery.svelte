<script lang="ts">
	import ImapDeliveryForm from '$lib/components/settings/imap-delivery-form.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import { REPORT_DELIVERY_INTRO, reportingModeCopy } from '$lib/reporting-modes';
	import type { SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/** How the domains' reports reach the mailbox; Continue saves the choice. */
	let {
		data,
		readonly = false,
		onBack,
		onNext
	}: { data: SetupData; readonly?: boolean; onBack: () => void; onNext: () => void } = $props();

	const reporting = $derived(data.settings.reporting);
	const copy = $derived(reportingModeCopy(reporting.reporting_mode));
</script>

<div class="space-y-4">
	{#if readonly}
		<p class="text-muted-foreground text-xs leading-relaxed">{REPORT_DELIVERY_INTRO}</p>
		<p class="text-muted-foreground text-xs leading-relaxed">
			{#if data.settings.mailbox.configured && reporting.reporting_mode_saved}
				<span class="text-foreground font-medium">{copy.title}</span>: {copy.fitsIf}
			{:else}
				Not chosen yet: every domain sends to the mailbox's address (central mailbox), or domains use addresses of their
				own that forward to it.
			{/if}
			Change it under <SettingsLink to="mailbox">Settings → Mailbox</SettingsLink>.
		</p>
		<StepNav {onBack} label="Next" onContinue={onNext} />
	{:else if !data.settings.mailbox.configured}
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
