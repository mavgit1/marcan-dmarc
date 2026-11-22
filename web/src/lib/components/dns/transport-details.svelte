<script module lang="ts">
	import type { BimiCheckResult } from '#lib/types/BimiCheckResult.ts';
	import type { LookupStatus } from '#lib/types/LookupStatus.ts';
	import type { MtaStsCheckResult } from '#lib/types/MtaStsCheckResult.ts';
	import type { TlsRptCheckResult } from '#lib/types/TlsRptCheckResult.ts';
	import DnsRawRecord from '#lib/components/dns-raw-record.svelte';
	import LookupBadge from './lookup-badge.svelte';
	import { formatInt } from '#lib/format.ts';
	import { policyFetchLabel } from '#lib/dns.ts';
	import { cn } from '#lib/utils.ts';

	/**
	 * The body of each transport check, exported so the DNS inspector draws the same facts in
	 * its own panels: the name queried with its raw DNS answer, the record, and what was parsed
	 * from it. Verdicts stay with the caller (a card badge here, a panel header there).
	 */
	export { mtaStsDetails, tlsRptDetails, bimiDetails };
</script>

<script lang="ts">
	import type { DnsCheckResponse } from '#lib/types/DnsCheckResponse.ts';
	import StatusBadge from '#lib/components/status-badge.svelte';
	import { bimiStatus, CHECK_FAILED, mtaStsStatus, recordStatus, type Verdict } from '#lib/dns.ts';
	import type { Snippet } from 'svelte';
	import DetailHeading from './detail-heading.svelte';

	let { dns }: { dns: DnsCheckResponse | null } = $props();
</script>

{#snippet nameLine(name: string, lookup: LookupStatus)}
	<div class="flex flex-wrap items-center gap-2 text-xs">
		<span class="font-mono font-medium break-all">{name}</span>
		<LookupBadge {lookup} />
	</div>
{/snippet}

{#snippet mtaStsDetails(m: MtaStsCheckResult)}
	{@const fields = [
		{ label: 'Policy URL', value: m.policy_url || '—', mono: !!m.policy_url },
		{ label: 'Policy fetch', value: policyFetchLabel(m.fetch), mono: false },
		{
			label: 'Max age',
			value: m.policy?.max_age != null ? `${formatInt(m.policy.max_age)} seconds` : '—',
			mono: false
		},
		{ label: 'MX host patterns', value: m.policy?.mx.join(', ') || '—', mono: !!m.policy?.mx.length }
	]}
	<div class="@container space-y-3">
		{@render nameLine(m.txt_name, m.txt_lookup)}
		{#if m.txt_record}
			<DnsRawRecord value={m.txt_record} label="MTA-STS record" />
		{/if}
		<div class="grid gap-2 @md:grid-cols-2 text-xs rounded-md border border-border bg-background p-3">
			{#each fields as field (field.label)}
				<div>
					<span class="text-muted-foreground text-[11px]">{field.label}</span>
					<p class={cn('font-medium text-foreground mt-0.5 break-all', field.mono ? 'font-mono' : 'tabular-nums')}>
						{field.value}
					</p>
				</div>
			{/each}
		</div>
	</div>
{/snippet}

{#snippet tlsRptDetails(t: TlsRptCheckResult)}
	<div class="space-y-3">
		{@render nameLine(t.name, t.lookup)}
		<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
		{#each t.records as rec}
			<DnsRawRecord value={rec} label="TLS-RPT record" />
		{/each}
	</div>
{/snippet}

{#snippet bimiDetails(b: BimiCheckResult)}
	<div class="space-y-3">
		{@render nameLine(b.name, b.lookup)}
		<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
		{#each b.records as rec}
			<DnsRawRecord value={rec} label="BIMI record" />
		{/each}
		{#if b.records.length > 0}
			<div class="text-xs">
				<p class="text-muted-foreground">
					Logo location:
					<span class={cn('text-foreground', b.parsed?.logo_url && 'font-mono break-all')}>
						{b.parsed?.logo_url || '—'}
					</span>
				</p>
				<p class="text-muted-foreground mt-0.5">
					Certificate (VMC):
					<span class={cn('text-foreground', b.parsed?.authority_url && 'font-mono break-all')}>
						{b.parsed?.authority_url || '—'}
					</span>
				</p>
			</div>
		{/if}
	</div>
{/snippet}

{#snippet card(title: string, status: Verdict, body: Snippet, note?: string)}
	<div class="rounded-lg border border-border p-3.5 bg-muted/10">
		<div class="flex items-center justify-between mb-2">
			<div class="flex items-center gap-2">
				<DetailHeading>{title}</DetailHeading>
				{#if note}
					<span class="text-[11px] text-muted-foreground font-normal">{note}</span>
				{/if}
			</div>
			<StatusBadge tone={status.tone} class={cn('text-xs font-medium', status.mono && 'font-mono')}>
				{status.label}
			</StatusBadge>
		</div>
		{@render body()}
	</div>
{/snippet}

{#snippet mtaStsBody()}
	{#if dns}{@render mtaStsDetails(dns.mta_sts)}{/if}
{/snippet}

{#snippet tlsRptBody()}
	{#if dns}{@render tlsRptDetails(dns.tls_rpt)}{/if}
{/snippet}

{#snippet bimiBody()}
	{#if dns}{@render bimiDetails(dns.bimi)}{/if}
{/snippet}

<div class="space-y-4">
	{@render card('MTA-STS (RFC 8461)', dns ? mtaStsStatus(dns.mta_sts) : CHECK_FAILED, mtaStsBody)}
	{@render card(
		'TLS-RPT (RFC 8460)',
		dns ? recordStatus(dns.tls_rpt.records, dns.tls_rpt.lookup) : CHECK_FAILED,
		tlsRptBody
	)}
	{@render card('BIMI', dns ? { ...bimiStatus(dns.bimi), tone: 'neutral' } : CHECK_FAILED, bimiBody, 'Not scored')}
</div>
