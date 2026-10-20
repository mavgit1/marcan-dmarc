<script lang="ts">
	import type { SpfCheckResult } from '$lib/types/SpfCheckResult';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import SpfIncludeTree from '$lib/components/spf-include-tree.svelte';
	import { dnsLookupMeasured, spfLookupsLabel, spfTrailing, spfVoidLookupsLabel } from '$lib/dns';
	import { statusFg } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import DetailHeading from './detail-heading.svelte';

	let { check, domain }: { check: SpfCheckResult | undefined; domain: string } = $props();

	const trailing = $derived(spfTrailing(check));
	const budget = $derived([
		{
			label: 'DNS lookups',
			value: check ? spfLookupsLabel(check) : '—',
			tone: check?.exceeds_lookup_limit ? statusFg.fail : ''
		},
		{ label: 'Void lookups', value: check ? spfVoidLookupsLabel(check) : '—', tone: '' },
		{ label: 'Trailing all', value: trailing.label, tone: statusFg[trailing.tone], mono: trailing.mono }
	]);
</script>

<div class="space-y-4">
	<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
	{#each check?.records ?? [] as record}
		<DnsRawRecord value={record} label="Published SPF record" />
	{:else}
		{#if check && !dnsLookupMeasured(check.lookup)}
			<div class="rounded-md border border-border bg-muted/10 p-3 text-xs text-muted-foreground">
				Check failed: the lookup of <span class="font-mono">{domain}</span> returned {check.lookup}.
			</div>
		{:else}
			<div class="rounded-md border border-status-fail-border bg-status-fail-bg/40 p-3 text-xs text-status-fail-fg">
				No SPF record published at <span class="font-mono font-medium">{domain}</span>.
			</div>
		{/if}
	{/each}

	<div class="rounded-lg border border-border p-3.5 bg-muted/10">
		<DetailHeading class="mb-2">Lookup budget (RFC 7208)</DetailHeading>
		<div class="grid gap-3 sm:grid-cols-3 text-xs">
			{#each budget as item (item.label)}
				<div class="rounded-md border border-border bg-background p-2.5">
					<span class="text-muted-foreground text-[11px]">{item.label}</span>
					<p
						class={cn(
							'mt-0.5 text-sm font-semibold [overflow-wrap:anywhere]',
							item.mono ? 'font-mono' : 'tabular-nums',
							item.tone || 'text-foreground'
						)}
					>
						{item.value}
					</p>
				</div>
			{/each}
		</div>
	</div>

	{#if check?.include_tree}
		<div>
			<DetailHeading class="mb-1">Include tree</DetailHeading>
			<p class="text-muted-foreground text-xs mb-3">Own = lookups for that node; subtree = own plus all descendants.</p>
			<SpfIncludeTree node={check.include_tree} />
		</div>
	{/if}
</div>
