<script lang="ts">
	import MailboxSync from '$lib/components/settings/mailbox-sync.svelte';
	import { FIRST_REPORT_ETA } from '$lib/report-address';
	import { importSummary } from '$lib/imap-progress';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { onMount } from 'svelte';
	import { importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The import of the reports already in the folder, from the shared health poll (every 3 s
	 * while it runs). Continue is always open: the import keeps running behind the next steps.
	 */
	let { data, onBack, onNext }: { data: SetupData; onBack: () => void; onNext: () => void } = $props();

	// A pass that just started shows at once rather than at the next 30 s poll.
	onMount(() => void systemHealth.poll());

	const health = $derived(data.health);
	const running = $derived(importing(data));
	const summary = $derived(importSummary(health));
	const configured = $derived(data.settings.mailbox.configured);
</script>

<div class="space-y-4">
	{#if !configured}
		<p class="text-muted-foreground text-xs">Connect the mailbox first; the import starts once it is saved.</p>
	{:else}
		<p class="text-sm font-medium tabular-nums" role="status" aria-live="polite">
			{#if summary}
				{summary}
			{:else if health.imap_last_success_at != null}
				The folder holds no DMARC reports yet.
			{:else}
				The import starts with the next mailbox poll.
			{/if}
		</p>
		{#if !running && health.imap_last_success_at != null && health.total_reports === 0}
			<p class="text-muted-foreground text-xs">
				Receivers send them once a domain's DMARC record names the report address, usually within {FIRST_REPORT_ETA}.
			</p>
		{/if}
		<MailboxSync {health} mailboxConfigured={configured} />
	{/if}
	<StepNav {onBack} onContinue={onNext} hint={running ? 'You can continue, it keeps running.' : undefined} />
</div>
