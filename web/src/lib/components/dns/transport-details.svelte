<script lang="ts">
	import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { formatInt } from '$lib/format';
	import { bimiStatus, CHECK_FAILED, mtaStsStatus, recordStatus, type Verdict } from '$lib/dns';
	import { cn } from '$lib/utils.js';
	import type { Snippet } from 'svelte';
	import DetailHeading from './detail-heading.svelte';

	let {
		dns,
		domain
	}: {
		dns: DnsCheckResponse | null;
		domain: string;
	} = $props();

	const mtaSts = $derived(dns?.mta_sts);
	const tlsRptRecords = $derived(dns?.tls_rpt?.records ?? []);
	const bimi = $derived(dns?.bimi);
	const bimiRecords = $derived(bimi?.records ?? []);
	const policy = $derived(mtaSts?.policy);
	const mtaStsFields = $derived([
		{ label: 'Policy URL', value: mtaSts?.policy_url || '—', mono: !!mtaSts?.policy_url },
		{
			label: 'Max age',
			value: policy?.max_age != null ? `${formatInt(policy.max_age)} seconds` : '—',
			mono: false
		},
		{ label: 'MX host patterns', value: policy?.mx?.join(', ') || '—', mono: !!policy?.mx?.length }
	]);
</script>

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
	{#if mtaSts?.txt_record}
		<div class="mb-3">
			<DnsRawRecord value={mtaSts.txt_record} label="MTA-STS record" />
		</div>
	{/if}
	<div class="grid gap-2 sm:grid-cols-3 text-xs rounded-md border border-border bg-background p-3">
		{#each mtaStsFields as field (field.label)}
			<div>
				<span class="text-muted-foreground text-[11px]">{field.label}</span>
				<p class={cn('font-medium text-foreground mt-0.5 break-all', field.mono ? 'font-mono' : 'tabular-nums')}>
					{field.value}
				</p>
			</div>
		{/each}
	</div>
{/snippet}

{#snippet tlsRptBody()}
	<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
	{#each tlsRptRecords as rec}
		<DnsRawRecord value={rec} label="TLS-RPT record" />
	{:else}
		<p class="text-xs text-muted-foreground">No record at <span class="font-mono">_smtp._tls.{domain}</span></p>
	{/each}
{/snippet}

{#snippet bimiBody()}
	<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
	{#each bimiRecords as rec}
		<DnsRawRecord value={rec} label="BIMI record" />
	{:else}
		<p class="text-xs text-muted-foreground">No record at <span class="font-mono">default._bimi.{domain}</span></p>
	{/each}
	{#if bimiRecords.length > 0}
		<div class="mt-2 text-xs">
			<p class="text-muted-foreground">
				Logo location:
				<span class={cn('text-foreground', bimi?.parsed?.logo_url && 'font-mono break-all')}>
					{bimi?.parsed?.logo_url || '—'}
				</span>
			</p>
			<p class="text-muted-foreground mt-0.5">
				Certificate (VMC):
				<span class={cn('text-foreground', bimi?.parsed?.authority_url && 'font-mono break-all')}>
					{bimi?.parsed?.authority_url || '—'}
				</span>
			</p>
		</div>
	{/if}
{/snippet}

<div class="space-y-4">
	{@render card('MTA-STS (RFC 8461)', mtaSts ? mtaStsStatus(mtaSts) : CHECK_FAILED, mtaStsBody)}
	{@render card(
		'TLS-RPT (RFC 8460)',
		dns ? recordStatus(dns.tls_rpt.records, dns.tls_rpt.lookup) : CHECK_FAILED,
		tlsRptBody
	)}
	{@render card('BIMI', dns ? { ...bimiStatus(dns.bimi), tone: 'neutral' } : CHECK_FAILED, bimiBody, 'Not scored')}
</div>
