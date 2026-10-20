<script lang="ts">
	import type { DmarcCheckResult } from '$lib/types/DmarcCheckResult';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { authVerdict, CHECK_FAILED, dnsLookupMeasured } from '$lib/dns';
	import { statusFg } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import DetailHeading from './detail-heading.svelte';

	let {
		check,
		domain,
		loading = false,
		onRefresh
	}: {
		check: DmarcCheckResult | undefined;
		domain: string;
		loading?: boolean;
		onRefresh?: () => void;
	} = $props();

	const external = $derived(check?.external_authorization ?? []);
	/** Tags as published; an absent tag reads "—" (its default is not shown). */
	const tags = $derived.by(() => {
		const p = check?.parsed;
		if (!p) return [];
		const tag = (name: string, value: string | number | null) => (value == null ? '—' : `${name}=${value}`);
		return [
			{ dt: 'Policy', dd: tag('p', p.p) },
			{ dt: 'Subdomain policy', dd: tag('sp', p.sp) },
			{ dt: 'Percentage', dd: tag('pct', p.pct_raw ?? p.pct) },
			{
				dt: 'Alignment',
				dd: p.adkim == null && p.aspf == null ? '—' : [tag('adkim', p.adkim), tag('aspf', p.aspf)].join(' ')
			},
			{ dt: 'Aggregate reports', dd: p.rua.map((r) => r.raw).join(', ') || '—', wide: true },
			{ dt: 'Failure reports', dd: p.ruf.map((r) => r.raw).join(', ') || '—', wide: true }
		];
	});
</script>

<div class="space-y-4">
	<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
	{#each check?.records ?? [] as record}
		<DnsRawRecord value={record} label="Published DMARC record" />
	{:else}
		{#if check && !dnsLookupMeasured(check.lookup)}
			<div class="rounded-md border border-border bg-muted/10 p-3 text-xs text-muted-foreground">
				Check failed: the lookup of <span class="font-mono">_dmarc.{domain}</span> returned {check.lookup}.
			</div>
		{:else}
			<div class="rounded-md border border-status-fail-border bg-status-fail-bg/40 p-3 text-xs text-status-fail-fg">
				No DMARC record published at <span class="font-mono font-medium">_dmarc.{domain}</span>.
			</div>
		{/if}
	{/each}

	{#if tags.length > 0}
		<div>
			<DetailHeading class="mb-2">Parsed DMARC tags</DetailHeading>
			<dl class="grid gap-2 text-xs sm:grid-cols-4 rounded-lg border border-border p-3 bg-muted/10">
				{#each tags as tag (tag.dt)}
					<div class={cn(tag.wide && 'sm:col-span-2')}>
						<dt class="text-muted-foreground text-[11px]">{tag.dt}</dt>
						<dd class={cn('text-foreground', tag.dd !== '—' && 'font-mono', tag.wide && 'break-all')}>{tag.dd}</dd>
					</div>
				{/each}
			</dl>
		</div>
	{/if}

	<div class="rounded-lg border border-border p-3.5">
		<div class="flex items-center justify-between mb-2">
			<DetailHeading>External report destinations</DetailHeading>
		</div>
		<p class="text-xs text-muted-foreground mb-3">
			Authorization record (RFC 7489 §7.1) at <span class="font-mono text-foreground"
				>&lt;domain&gt;._report._dmarc.&lt;dest-domain&gt;</span
			>.
		</p>

		<div class="space-y-2">
			<!-- eslint-disable-next-line svelte/require-each-key -- rua and ruf can name the same URI -->
			{#each external as auth}
				{@const status = authVerdict(auth.verdict)}
				<div
					class="flex flex-col sm:flex-row sm:items-center justify-between gap-2 rounded-md border border-border bg-background p-2.5 text-xs"
				>
					<div class="min-w-0">
						<div class="flex items-center gap-2">
							<span class="font-mono font-medium text-foreground">{auth.host}</span>
							<span class="font-mono text-muted-foreground">({auth.uri})</span>
						</div>
						<div class="text-[11px] text-muted-foreground mt-0.5">
							Record: <span class="font-mono">{auth.auth_record_name}</span>
						</div>
					</div>
					<div class="flex items-center gap-2 shrink-0">
						<span class={cn('font-medium', statusFg[status.tone])}>{status.label}</span>
						{#if status === CHECK_FAILED && onRefresh}
							<Button
								variant="ghost"
								size="sm"
								class="h-6 px-1.5 text-[11px] gap-1 text-muted-foreground hover:text-foreground"
								onclick={onRefresh}
								disabled={loading}
							>
								<RefreshCwIcon class={cn('size-3', loading && 'animate-spin')} />
								Re-check
							</Button>
						{/if}
					</div>
				</div>
			{:else}
				<div class="rounded-md border border-border bg-muted/10 p-2.5 text-xs text-muted-foreground">
					No external report addresses.
				</div>
			{/each}
		</div>
	</div>
</div>
