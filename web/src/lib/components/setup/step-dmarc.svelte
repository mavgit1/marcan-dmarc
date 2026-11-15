<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import ReportingSetup from '$lib/components/reporting-setup/reporting-setup.svelte';
	import type { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import { importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The DNS records that send each domain's reports here, every domain's status as the server
	 * decided it; a record to publish names the first receiving account's address. Continue is
	 * always open: DNS changes take time to show.
	 */
	let {
		data,
		reporting,
		onBack,
		onNext
	}: {
		data: SetupData;
		reporting: ReportingSetupState;
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
	{#if data.domains.length === 0 && reporting.data?.default_address}
		<p class="text-muted-foreground text-xs">
			No domains yet, so the record is shown for any domain, <InlineCode>{'<domain>'}</InlineCode> standing for its name.
		</p>
	{/if}
	<ReportingSetup exampleBelow importing={running} setup={reporting}>
		{#snippet addressHint()}
			No account receives reports yet, so there is no record to publish: add one in the Accounts step.
		{/snippet}
	</ReportingSetup>
	<StepNav {onBack} onContinue={onNext} />
</div>
