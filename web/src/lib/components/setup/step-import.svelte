<script lang="ts">
	import ImportProgress from '#lib/components/import-progress.svelte';
	import MailboxSync from '#lib/components/settings/mailbox-sync.svelte';
	import { relativeTime } from '#lib/format.ts';
	import { FIRST_REPORT_ETA } from '#lib/report-address.ts';
	import { importSummary } from '#lib/pass-progress.ts';
	import { MAILBOX_LABEL } from '#lib/pipeline.ts';
	import { systemHealth } from '#lib/state/system-health.svelte.ts';
	import { statusDot, statusText } from '#lib/tokens.ts';
	import { onMount } from 'svelte';
	import { accountReady, importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The import of the reports already in each account's folder, from the shared health poll
	 * (every 3 s while it runs): one line per receiving account, the one being read with its
	 * progress. Continue is always open: the import keeps running behind the next steps.
	 */
	let { data, onBack, onNext }: { data: SetupData; onBack: () => void; onNext: () => void } = $props();

	// A pass that just started shows at once rather than at the next 30 s poll.
	onMount(() => void systemHealth.poll());

	const health = $derived(data.health);
	const running = $derived(importing(data));
	const summary = $derived(importSummary(health));
	const configured = $derived(accountReady(data.settings));
</script>

<div class="space-y-4">
	{#if !configured}
		<p class="text-muted-foreground text-xs">Add an account first; its import starts once it is saved.</p>
	{:else}
		<p class="text-sm font-medium tabular-nums" role="status" aria-live="polite">
			{#if summary}
				{summary}
			{:else if health.receive_last_success_at != null}
				The folders hold no DMARC reports yet.
			{:else if health.accounts.some((a) => a.last_error)}
				Not every account could be read yet: each one's error is below.
			{:else}
				The import starts with the next check.
			{/if}
		</p>
		<ul class="divide-border divide-y rounded-lg border text-xs" aria-label="Accounts">
			{#each health.accounts as a (a.id)}
				{@const progress = health.pass_progress?.account_id === a.id ? health.pass_progress : null}
				<li class="space-y-1.5 px-3 py-2">
					<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
						<span class="min-w-0 font-medium break-words">{a.name}</span>
						<span class="text-muted-foreground min-w-0 break-all">{a.address}</span>
					</div>
					{#if progress}
						<ImportProgress {progress} />
					{:else}
						<p class="text-muted-foreground flex items-center gap-1.5">
							<span class={['size-1.5 shrink-0 rounded-full', statusDot[a.status]]}></span>
							{MAILBOX_LABEL[a.state]}{a.last_check_at != null ? `, checked ${relativeTime(a.last_check_at)}` : ''}
						</p>
						{#if a.last_error}
							<p class={['break-words', statusText.fail]}>{a.last_error}</p>
						{/if}
					{/if}
				</li>
			{/each}
		</ul>
		{#if !running && health.receive_last_success_at != null && health.total_reports === 0}
			<p class="text-muted-foreground text-xs">
				Receivers send them once a domain's DMARC record names a report address, usually within {FIRST_REPORT_ETA}.
			</p>
		{/if}
		<div class="flex justify-end">
			<MailboxSync {health} mailboxConfigured={configured} variant="button" />
		</div>
	{/if}
	<StepNav {onBack} onContinue={onNext} hint={running ? 'You can continue, it keeps running.' : undefined} />
</div>
