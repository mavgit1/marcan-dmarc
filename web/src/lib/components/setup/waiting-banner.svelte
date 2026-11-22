<script lang="ts">
	import type { SystemHealthResponse } from '#lib/types/SystemHealthResponse.ts';
	import MailboxSync from '#lib/components/settings/mailbox-sync.svelte';
	import { resolve } from '$app/paths';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { FIRST_REPORT_ETA } from '#lib/report-address.ts';
	import { systemHealth } from '#lib/state/system-health.svelte.ts';
	import HourglassIcon from '@lucide/svelte/icons/hourglass';

	/** Slim dashboard notice while domains exist but no report has arrived yet. */
	let { health, mailboxConfigured }: { health: SystemHealthResponse; mailboxConfigured: boolean } = $props();

	const progress = $derived(systemHealth.current?.pass_progress ?? health.pass_progress);
</script>

<div class="bg-status-info-bg/50 flex items-start gap-x-4 gap-y-2 rounded-lg border px-3 py-2 text-xs" role="status">
	<HourglassIcon class="text-status-info-fg mt-0.5 size-4 shrink-0" />
	<div class="min-w-0 flex-1">
		<p class="font-medium">
			{#if !mailboxConfigured}
				No account receives reports yet
			{:else if progress}
				{progress.phase === 'importing' ? 'Importing reports from the mailbox' : 'Checking the mailbox'}
			{:else}
				Waiting for the first DMARC report
			{/if}
		</p>
		<MailboxSync
			{health}
			{mailboxConfigured}
			variant="inline"
			note={mailboxConfigured
				? `Usually within ${FIRST_REPORT_ETA}.`
				: 'Add the account your DMARC records send reports to.'}
		>
			{#snippet actions()}
				<Button variant={mailboxConfigured ? 'ghost' : 'default'} size="sm" href={resolve('/setup')}>
					{mailboxConfigured ? 'Open setup' : 'Add an account'}
				</Button>
			{/snippet}
		</MailboxSync>
	</div>
</div>
