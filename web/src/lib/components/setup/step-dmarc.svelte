<script lang="ts">
	import ReportingSetup from '$lib/components/reporting-setup/reporting-setup.svelte';
	import type { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import { importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The DNS records that send each domain's reports here, every domain's status as the server
	 * decided it. Continue is always open: DNS changes take time to show.
	 */
	let {
		data,
		reporting,
		readonly = false,
		onBack,
		onNext
	}: {
		data: SetupData;
		reporting: ReportingSetupState;
		readonly?: boolean;
		onBack: () => void;
		onNext: () => void;
	} = $props();

	const running = $derived(importing(data));
</script>

<div class="space-y-4">
	{#if running}
		<p class="text-xs font-medium" role="status">
			Still importing: more domains may appear here, and a domain's status can change once its reports arrive.
		</p>
	{/if}
	{#if data.domains.length === 0}
		<p class="text-muted-foreground text-xs">No domains yet, so here is an example.</p>
	{/if}
	<ReportingSetup
		exampleBelow
		importing={running}
		setup={reporting}
		reportAddress={data.settings.reporting.report_address}
		reportingMode={data.settings.reporting.reporting_mode}
	>
		{#snippet addressHint()}
			No report address yet, so the records show a placeholder: connect the mailbox in the Mailbox step.
		{/snippet}
	</ReportingSetup>
	<StepNav {onBack} label={readonly ? 'Next' : 'Continue'} onContinue={onNext} />
</div>
