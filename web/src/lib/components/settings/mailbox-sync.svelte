<script lang="ts">
	import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
	import ImportProgress from '$lib/components/import-progress.svelte';
	import { relativeTime } from '$lib/format';
	import { lastErrorText, lastPollText } from '$lib/pipeline';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import type { Snippet } from 'svelte';
	import { Action } from '../action.svelte';
	import BusyButton from '../busy-button.svelte';
	import FeedbackAlert from './feedback-alert.svelte';

	/**
	 * Mailbox poll state (running pass, last poll, last error) and the "Sync now" button, for the
	 * setup (`panel`), the dashboard banner (`inline`) and the header's status popover
	 * (`button`: the button only; the popover shows the poll state itself).
	 */
	let {
		health,
		mailboxConfigured = health.imap_configured,
		variant = 'panel',
		note,
		actions
	}: {
		health: SystemHealthResponse;
		mailboxConfigured?: boolean;
		variant?: 'panel' | 'inline' | 'button';
		/** Inline: text before the last-poll line. */
		note?: string;
		/** Rendered beside the Sync button (`inline`: after it, else before it). */
		actions?: Snippet;
	} = $props();

	const sync = new Action();
	/** The shared poll is live (3 s while a pass runs); `health` may be a parent's snapshot. */
	const live = $derived(systemHealth.current ?? health);
	const progress = $derived(live.imap_progress);
	/** While a pass runs only its progress shows; poll history and Sync return when it ends. */
	const importing = $derived(mailboxConfigured && progress != null);
	const error = $derived(lastErrorText(live));
	const pollAt = $derived(live.last_poll.at);

	function onSync() {
		sync.run(() => systemHealth.startSync());
	}
</script>

{#snippet button()}
	{#if mailboxConfigured && !importing}
		<BusyButton
			type="button"
			variant="outline"
			size="sm"
			busy={sync.busy || progress != null}
			busyLabel="Syncing…"
			onclick={onSync}
		>
			<RefreshCwIcon class="mr-1.5 size-3.5" />
			Sync now
		</BusyButton>
	{/if}
{/snippet}

{#if variant === 'button'}
	<div class="flex items-center gap-2">
		{#if sync.feedback && !sync.feedback.ok}
			<span class="text-status-fail-fg text-[11px]">{sync.feedback.message}</span>
		{/if}
		{@render button()}
	</div>
{:else if variant === 'inline'}
	<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
		<div class="min-w-0 flex-1 space-y-1">
			{#if importing && progress}
				<ImportProgress {progress} class="max-w-md" />
			{:else}
				<p class="text-muted-foreground">
					{note}
					{#if mailboxConfigured}{lastPollText(live)}.{/if}
				</p>
				{#if error}
					<p class="text-status-fail-fg text-[11px] break-words">{error}</p>
				{/if}
			{/if}
			{#if sync.feedback}
				<p class={sync.feedback.ok ? 'text-muted-foreground' : 'text-status-fail-fg'}>{sync.feedback.message}</p>
			{/if}
		</div>
		<div class="flex items-center gap-2">
			{@render button()}
			{@render actions?.()}
		</div>
	</div>
{:else}
	<div class="space-y-3">
		{#if variant === 'panel' && importing && progress}
			<ImportProgress {progress} class="rounded-lg border px-3 py-2" />
		{:else if variant === 'panel'}
			<dl class="divide-border divide-y rounded-lg border text-xs">
				<div class="flex justify-between gap-3 px-3 py-2">
					<dt class="text-muted-foreground">Last poll</dt>
					<dd>
						{#if !mailboxConfigured}
							Not set up
						{:else if pollAt != null}
							{relativeTime(pollAt)}{live.last_poll.failed ? ' (failed)' : ''}
						{:else}
							Not yet
						{/if}
					</dd>
				</div>
				<div class="flex justify-between gap-3 px-3 py-2">
					<dt class="text-muted-foreground">Last successful poll</dt>
					<dd>{live.imap_last_success_at ? relativeTime(live.imap_last_success_at) : 'Never'}</dd>
				</div>
				{#if error}
					<div class="space-y-0.5 px-3 py-2">
						<dt class="text-status-fail-fg font-medium">Last error</dt>
						<dd class="text-status-fail-fg text-[11px] break-words">{error}</dd>
					</div>
				{/if}
			</dl>
		{/if}
		<!-- Only a sync that did not start has feedback; its outcome shows in the poll state. -->
		<FeedbackAlert feedback={sync.feedback} />
		<div class="flex flex-wrap items-center justify-between gap-2">
			{@render actions?.()}
			{@render button()}
		</div>
	</div>
{/if}
