<script lang="ts">
	import type { SpfCheckResult } from '#lib/types/SpfCheckResult.ts';
	import DnsRawRecord from '#lib/components/dns-raw-record.svelte';
	import SpfIncludeTree from '#lib/components/spf-include-tree.svelte';
	import { dnsLookupMeasured, spfLookupsLabel, spfTrailing, spfVoidLookupsLabel } from '#lib/dns.ts';
	import { statusFg } from '#lib/tokens.ts';
	import { cn } from '#lib/utils.ts';
	import DetailHeading from './detail-heading.svelte';

	/**
	 * The SPF record, its lookup budget and include tree, for the scorecard drawer and the DNS
	 * inspector. A failed TXT lookup shows only why; an incomplete include walk shows no counts
	 * (they would turn "unknown" into a number), only the tree as far as it got.
	 */
	let { check, domain }: { check: SpfCheckResult | undefined; domain: string } = $props();

	const trailing = $derived(spfTrailing(check));
	const budget = $derived([
		{
			label: 'DNS lookups',
			value: check ? spfLookupsLabel(check) : '—',
			tone: check?.exceeds_lookup_limit ? statusFg.fail : '',
			hint: !check ? '' : check.exceeds_lookup_limit ? 'Above the limit.' : 'Within the limit.'
		},
		{
			label: 'Void lookups',
			value: check ? spfVoidLookupsLabel(check) : '—',
			tone: '',
			hint: 'NXDOMAIN or empty answers while walking includes.'
		},
		{
			label: 'SPF ending',
			value: trailing.label,
			tone: statusFg[trailing.tone],
			mono: trailing.mono,
			hint: 'Applies to every sender not listed.'
		}
	]);
</script>

<div class="space-y-4">
	{#if check && !dnsLookupMeasured(check.lookup)}
		<div class="rounded-md border border-border bg-muted/10 p-3 text-xs text-muted-foreground">
			Check failed: the lookup of <span class="font-mono">{domain}</span> returned {check.lookup}.
		</div>
	{:else}
		<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
		{#each check?.records ?? [] as record}
			<DnsRawRecord value={record} label="Published SPF record" />
		{:else}
			<div class="rounded-md border border-status-fail-border bg-status-fail-bg/40 p-3 text-xs text-status-fail-fg">
				No SPF record published at <span class="font-mono font-medium">{domain}</span>.
			</div>
		{/each}

		{#if check && !check.complete}
			<div class="rounded-md border border-border bg-muted/10 p-3 text-xs text-muted-foreground">
				Some include targets did not answer, so the lookup count is unknown.
			</div>
		{:else}
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
							{#if item.hint}
								<p class="text-muted-foreground mt-1 text-[11px]">{item.hint}</p>
							{/if}
						</div>
					{/each}
				</div>
			</div>
		{/if}

		{#if check?.include_tree}
			<div>
				<DetailHeading class="mb-1">Include tree</DetailHeading>
				<p class="text-muted-foreground text-xs mb-3">
					Own = lookups for that node; subtree = own plus all descendants.
				</p>
				<SpfIncludeTree node={check.include_tree} />
			</div>
		{/if}
	{/if}
</div>
