<script lang="ts">
	import MailboxSync from '#lib/components/settings/mailbox-sync.svelte';
	import { plural } from '#lib/format.ts';
	import { FIRST_REPORT_ETA } from '#lib/report-address.ts';
	import NextSteps from './next-steps.svelte';
	import { accountReady, importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The end of the setup: what arrived so far, the optional next steps, and Finish (which needs
	 * an account that receives reports; the first report does not have to be here yet).
	 */
	let {
		data,
		finishing = false,
		onBack,
		onFinish
	}: {
		data: SetupData;
		finishing?: boolean;
		onBack: () => void;
		onFinish?: () => void;
	} = $props();

	const health = $derived(data.health);
	const configured = $derived(accountReady(data.settings));
</script>

<div class="space-y-4">
	{#if !configured}
		<p class="text-muted-foreground text-xs">No account receives reports yet.</p>
	{:else if health.total_reports > 0}
		<p class="text-xs">
			{plural(health.total_reports, 'report')} received{importing(data) ? ' so far; the import keeps running' : ''}.
		</p>
	{:else}
		<p class="text-muted-foreground text-xs leading-relaxed">
			No report yet. The first ones usually arrive {FIRST_REPORT_ETA} after a domain's DMARC record names a report address.
		</p>
		<MailboxSync {health} mailboxConfigured={configured} />
	{/if}
	<div class="border-t pt-4">
		<NextSteps />
	</div>
	<StepNav
		{onBack}
		label="Finish setup"
		onContinue={onFinish}
		busy={finishing}
		disabled={!configured}
		hint={configured ? undefined : 'Add an account in the Accounts step first.'}
	/>
</div>
