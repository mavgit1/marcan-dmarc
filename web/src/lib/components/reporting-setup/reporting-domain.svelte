<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import { apiErrorMessage } from '$lib/http';
	import type { ReportingDomain } from '$lib/types/ReportingDomain';
	import type { ReportingMode } from '$lib/types/ReportingMode';
	import CopyButton from '$lib/components/copy-button.svelte';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { relativeTime } from '$lib/format';
	import { FIRST_REPORT_ETA, REPORTING_STATUS_LABEL } from '$lib/report-address';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import type { ReportingSetupState } from './reporting-setup-state.svelte';

	/**
	 * One monitored domain's reporting setup, as the server decided it (`row.state`): this only
	 * words and colours it, and shows the records to publish (`record_to_publish`, written by the
	 * server). A domain whose reports arrive is one line until its record is shown.
	 */
	let {
		row,
		setup,
		mode,
		mailboxDomain
	}: {
		row: ReportingDomain;
		/** Looks the row up again (Re-check, Show record). */
		setup: ReportingSetupState;
		mode: ReportingMode;
		mailboxDomain: string;
	} = $props();

	const reporting = $derived(row.state === 'reporting');
	let expanded = $state(false);
	const compact = $derived(reporting && !expanded);
	const busy = $derived(setup.busy.has(row.id));
	const failed = $derived(setup.failed.get(row.id) ?? null);

	// A reporting row is looked up only once its record is shown.
	$effect(() => {
		if (expanded && reporting && !row.dmarc) void setup.lookup(row.id, false);
	});

	const check = $derived(row.dmarc);
	const hasRecord = $derived(!!check && check.record_count > 0 && !!check.parsed);
	const current = $derived(hasRecord && check ? check.records[0] : '');
	const wildcard = $derived(`*._report._dmarc.${mailboxDomain}`);
</script>

{#snippet mailboxAuthNote(also = true)}
	{#if row.mailbox_auth_missing}
		<p class="text-status-fail-fg text-xs">
			{also ? 'Also publish' : 'Publish'} this on <InlineCode>{mailboxDomain}</InlineCode> so it accepts reports for {row.domain},
			or the wildcard <InlineCode>{wildcard}</InlineCode> for every domain:
		</p>
		<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,8rem)]">
			<DnsRawRecord value={`${row.domain}._report._dmarc.${mailboxDomain}`} label="Host (TXT)" />
			<DnsRawRecord value="v=DMARC1" label="Value" />
		</div>
	{/if}
{/snippet}

{#snippet addressMissing()}
	<p class="text-muted-foreground text-xs">
		Add this mailbox's address to rua: replace the record with this value. The policy and every other tag stay as they
		are.
	</p>
	{#if row.record_to_publish}
		<DnsRawRecord value={row.record_to_publish} label={`New TXT value for _dmarc.${row.domain}`} />
	{/if}
	{@render mailboxAuthNote()}
{/snippet}

{#snippet currentRecord()}
	{#if check}
		<div class="flex flex-wrap items-center gap-2 text-xs">
			<span class="text-muted-foreground">Current policy</span>
			<PolicyBadge enforcement={check.enforcement} policy={check.parsed?.p} />
			{#if check.record_count > 1}
				<StatusBadge tone="fail" class="text-xs">{check.record_count} records published</StatusBadge>
			{/if}
		</div>
		<DnsRawRecord value={current} label="Current record" />
	{/if}
{/snippet}

<li class={compact ? 'px-3 py-2' : 'space-y-3 p-3'}>
	<div class="flex flex-wrap items-center justify-between gap-2">
		<div class="flex min-w-0 items-center gap-2">
			<span class="truncate font-mono text-sm font-medium">{row.domain}</span>
			<StatusBadge tone={row.status} class="text-xs">{REPORTING_STATUS_LABEL[row.state]}</StatusBadge>
		</div>
		<div class="flex items-center gap-1">
			{#if reporting}
				<Button variant="ghost" size="sm" onclick={() => (expanded = !expanded)}>
					{expanded ? 'Hide record' : 'Show record'}
				</Button>
			{:else}
				<BusyButton
					variant="ghost"
					size="sm"
					class="gap-1.5"
					{busy}
					busyLabel="Checking DNS…"
					onclick={() => setup.lookup(row.id, true)}
				>
					<RefreshCwIcon class="size-3.5" />
					Re-check
				</BusyButton>
			{/if}
		</div>
	</div>

	{#if failed}
		<p class="text-status-fail-fg text-xs" role="alert">{apiErrorMessage(failed)}</p>
	{/if}

	{#if compact}
		<!-- one line -->
	{:else if reporting}
		{#if check}
			{@render currentRecord()}
		{:else}
			<Skeleton class="h-16 w-full" />
		{/if}
	{:else if row.state === 'check_failed'}
		<p class="text-muted-foreground text-xs">
			Could not read <InlineCode>{`_dmarc.${row.domain}`}</InlineCode>{check ? `: DNS lookup ${check.lookup}` : ''}.
			Re-check in a moment.
		</p>
	{:else if row.state === 'no_record'}
		<p class="text-muted-foreground text-xs">
			Publish this TXT record; <InlineCode>{'p=none'}</InlineCode> only monitors and does not affect delivery.
			{#if mode === 'forwarded'}
				The rua address may be any address that forwards to this mailbox; this one is an example.
			{/if}
		</p>
		<div class="flex items-center gap-1 text-xs">
			<span class="text-muted-foreground">Host</span>
			<InlineCode>{`_dmarc.${row.domain}`}</InlineCode>
			<CopyButton value={`_dmarc.${row.domain}`} label="Copy host name" />
		</div>
		{#if row.record_to_publish}
			<DnsRawRecord value={row.record_to_publish} label="TXT value" />
		{/if}
		{@render mailboxAuthNote()}
	{:else}
		{@render currentRecord()}
		{#if row.state === 'not_authorized'}
			{@render mailboxAuthNote(false)}
		{:else if row.state === 'authorization_check_failed'}
			<p class="text-muted-foreground text-xs">
				Could not check whether <InlineCode>{mailboxDomain}</InlineCode> accepts reports for {row.domain}. Re-check in a
				moment.
			</p>
		{:else if row.state === 'address_missing'}
			{@render addressMissing()}
		{:else if row.names_report_address}
			<p class="text-muted-foreground text-xs">
				{#if row.last_report_at != null}
					Last report {relativeTime(row.last_report_at)}. The record is correct; reports resume when this domain sends
					mail again.
				{:else}
					Reports usually arrive within {FIRST_REPORT_ETA}.
				{/if}
			</p>
		{:else}
			<p class="text-muted-foreground text-xs">Make sure one of these addresses forwards to this mailbox:</p>
			<ul class="space-y-1 text-xs">
				{#each row.other_addresses as addr (addr)}
					<li><InlineCode>{addr}</InlineCode></li>
				{/each}
			</ul>
		{/if}
		{#if check && check.record_count > 1}
			<p class="text-status-fail-fg text-xs">
				Receivers ignore DMARC when more than one record is published. Keep a single one.
			</p>
		{/if}
	{/if}

	{#if !reporting}
		{#each row.refused as auth (auth.auth_record_name)}
			<div class="space-y-2 text-xs">
				<div class="flex flex-wrap items-center gap-2">
					<StatusBadge tone="fail" class="text-xs">Not authorized</StatusBadge>
					<span class="text-muted-foreground">
						<InlineCode>{auth.host}</InlineCode> does not accept reports for <InlineCode>{row.domain}</InlineCode> yet. Publish
						on
						<InlineCode>{auth.host}</InlineCode>:
					</span>
				</div>
				<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,8rem)]">
					<DnsRawRecord value={auth.auth_record_name} label="Host (TXT)" />
					<DnsRawRecord value="v=DMARC1" label="Value" />
				</div>
			</div>
		{/each}
	{/if}
</li>
