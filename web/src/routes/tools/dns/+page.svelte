<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getDnsBimi, getDnsDkim, getDnsDmarc, getDnsMtaSts, getDnsPtr, getDnsSpf, getDnsTlsRpt } from '$lib/api';
	import { apiErrorMessage } from '$lib/http';
	import type { LookupStatus } from '$lib/types/LookupStatus';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import DnsCheckPanel from '$lib/components/dns-check-panel.svelte';
	import DnsFindingsList from '$lib/components/dns-findings-list.svelte';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import LookupBadge from '$lib/components/dns/lookup-badge.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import SpfIncludeTree from '$lib/components/spf-include-tree.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import DetailHeading from '$lib/components/dns/detail-heading.svelte';
	import DmarcDetails from '$lib/components/dns/dmarc-details.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import {
		CHECK_FAILED,
		bimiStatus,
		dkimKey,
		dnsLookupMeasured,
		findingsVerdict,
		mtaStsStatus,
		policyFetchLabel,
		dmarcRecordStatus,
		recordStatus,
		selectorVerdict,
		spfLookupsLabel,
		spfTrailing,
		spfVoidLookupsLabel,
		worstVerdict,
		type Verdict
	} from '$lib/dns';
	import { enforcementLabel } from '$lib/health';
	import { plural } from '$lib/format';
	import { resource, type Resource } from '$lib/state/resource.svelte';
	import { listTokens, normalizeDomain } from '$lib/text';
	import { statusFg } from '$lib/tokens';
	import { cn } from '$lib/utils.js';

	/**
	 * Status, error and refreshing props for a `DnsCheckPanel`: idle before a domain is
	 * entered, a skeleton only for the first load (a re-check keeps the last result on
	 * screen and says it is checking), and a failed re-check shows its error.
	 */
	function panel(check: Resource<unknown>) {
		const status =
			check.loading && check.data === null ? 'loading' : check.error ? 'error' : check.data !== null ? 'ready' : 'idle';
		return {
			status,
			errorMessage: status === 'error' ? apiErrorMessage(check.error) : null,
			refreshing: check.loading && check.data !== null
		} as const;
	}

	let domainDraft = $state('');
	let selectorsDraft = $state('');
	let ipDraft = $state('');

	let activeDomain = $state('');
	/**
	 * DKIM selectors for the last run. DKIM keys cannot be discovered from DNS,
	 * and this tool must work for domains Marcan does not monitor, so it checks
	 * exactly what the user entered and never guesses. `null` = none entered.
	 */
	let pinnedSelectors = $state<string[] | null>(null);
	const dkimNeedsSelectors = $derived(activeDomain !== '' && pinnedSelectors == null);

	/** The IP of the last PTR lookup. */
	let activeIp = $state('');

	// Live checks keyed on what was entered: they have their own freshness (the server's
	// caches), so an app refresh does not re-run them; Inspect on the same input does.
	function domainCheck<T>(fetch: (domain: string) => Promise<T>): Resource<T> {
		return resource({
			key: () => activeDomain,
			load: () => (activeDomain ? fetch(activeDomain) : null),
			refetchOnRefresh: false
		});
	}
	const dmarc = domainCheck((d) => getDnsDmarc(d));
	const spf = domainCheck(getDnsSpf);
	const mtaSts = domainCheck(getDnsMtaSts);
	const tlsRpt = domainCheck(getDnsTlsRpt);
	const bimi = domainCheck(getDnsBimi);
	const dkim = resource({
		key: () => `${activeDomain}|${pinnedSelectors?.join(',') ?? ''}`,
		load: () => (activeDomain && pinnedSelectors ? getDnsDkim(activeDomain, pinnedSelectors) : null),
		refetchOnRefresh: false
	});
	const domainChecks = [dmarc, spf, mtaSts, tlsRpt, bimi, dkim];
	const ptr = resource({
		key: () => activeIp,
		load: () => (activeIp ? getDnsPtr(activeIp) : null),
		refetchOnRefresh: false
	});

	let urlSynced = false;

	const parseSelectorList = (raw: string | null | undefined) => listTokens(raw).map((s) => s.toLowerCase());

	function runDomainChecks(domain: string, selectors: string[] | null) {
		const pinned = selectors && selectors.length > 0 ? [...selectors] : null;
		const same = domain === activeDomain && (pinned?.join(',') ?? '') === (pinnedSelectors?.join(',') ?? '');
		activeDomain = domain;
		pinnedSelectors = pinned;
		// The same input again (Inspect, a retry) re-runs every check.
		if (same) for (const check of domainChecks) check.reload();
	}

	function syncUrl(domain: string, selectors: string[] | null) {
		const url = new URL(page.url);
		if (domain) url.searchParams.set('domain', domain);
		else url.searchParams.delete('domain');
		if (selectors && selectors.length > 0) {
			url.searchParams.set('selectors', selectors.join(','));
		} else {
			url.searchParams.delete('selectors');
		}
		const href = `${url.pathname}${url.search}${url.hash}`;
		void goto(resolve(href as '/'), { replaceState: true, keepFocus: true, noScroll: true });
	}

	function onInspectDomain(event: Event) {
		event.preventDefault();
		const domain = normalizeDomain(domainDraft);
		if (!domain) return;
		const selectors = parseSelectorList(selectorsDraft);
		const pinned = selectors.length > 0 ? selectors : null;
		domainDraft = domain;
		selectorsDraft = selectors.join(',');
		syncUrl(domain, pinned);
		runDomainChecks(domain, pinned);
	}

	function onInspectPtr(event: Event) {
		event.preventDefault();
		const ip = ipDraft.trim();
		if (!ip) return;
		if (ip === activeIp) ptr.reload();
		else activeIp = ip;
	}

	function retryDomain() {
		if (!activeDomain) return;
		runDomainChecks(activeDomain, pinnedSelectors);
	}

	// Deep-link: ?domain= and optional ?selectors=
	$effect(() => {
		const domainParam = page.url.searchParams.get('domain');
		const selectorsParam = page.url.searchParams.get('selectors');
		const domain = domainParam ? normalizeDomain(domainParam) : '';
		const selectors = parseSelectorList(selectorsParam);
		const pinned = selectors.length > 0 ? selectors : null;

		domainDraft = domainParam ? domain : domainDraft;
		if (selectorsParam != null) selectorsDraft = selectors.join(',');

		if (!domain) {
			urlSynced = true;
			return;
		}
		if (urlSynced && domain === activeDomain) return;
		urlSynced = true;
		runDomainChecks(domain, pinned);
	});
</script>

{#snippet verdictBadge(v: Verdict)}
	<StatusBadge tone={v.tone} class={cn(v.mono && 'font-mono')}>{v.label}</StatusBadge>
{/snippet}

{#snippet nameLine(name: string, lookup: LookupStatus)}
	<div class="flex flex-wrap items-center gap-2 text-xs">
		<span class="font-mono font-medium">{name}</span>
		<LookupBadge {lookup} />
	</div>
{/snippet}

<PageHeader
	title="DNS inspector"
	description="Live check of SPF, DKIM, DMARC, MTA-STS, TLS-RPT and BIMI records for any domain, including ones Marcan doesn't monitor."
>
	{#snippet actions()}
		<Button variant="outline" size="sm" href={resolve('/guide')}>
			<BookOpenIcon />
			Setup guide
		</Button>
	{/snippet}
</PageHeader>

<form
	class="mb-6 flex flex-col gap-3 rounded-lg border border-border bg-card p-4 sm:flex-row sm:flex-wrap sm:items-end"
	onsubmit={onInspectDomain}
>
	<div class="min-w-0 flex-1 space-y-1.5 sm:min-w-56">
		<label for="dns-domain" class="text-sm font-medium">Domain</label>
		<Input id="dns-domain" bind:value={domainDraft} placeholder="example.com" autocomplete="off" spellcheck={false} />
	</div>
	<div class="min-w-0 flex-1 space-y-1.5 sm:min-w-56">
		<label for="dns-selectors" class="text-sm font-medium">DKIM selectors</label>
		<Input
			id="dns-selectors"
			bind:value={selectorsDraft}
			placeholder="selector1, google, …"
			autocomplete="off"
			spellcheck={false}
		/>
	</div>
	<Button type="submit" disabled={normalizeDomain(domainDraft) === ''}>Inspect</Button>
</form>

{#if activeDomain}
	<p class="text-muted-foreground mb-4 text-sm">
		Showing live checks for <span class="text-foreground font-mono">{activeDomain}</span>.
	</p>
{/if}

<div class="grid gap-4">
	<DnsCheckPanel title="SPF" {...panel(spf)} onRetry={retryDomain} open={false}>
		{#snippet header()}
			{#if spf.data && spf.data.complete}
				{@const trailing = spfTrailing(spf.data)}
				<span class="tabular-nums">{spfLookupsLabel(spf.data)} lookups</span>
				{@render verdictBadge(worstVerdict(findingsVerdict(spf.data.findings), trailing))}
			{:else if spf.data}
				{@render verdictBadge(CHECK_FAILED)}
			{/if}
		{/snippet}
		{#if spf.data && !dnsLookupMeasured(spf.data.lookup)}
			<p class="text-muted-foreground text-sm">
				The TXT lookup for <span class="font-mono">{spf.data.domain}</span> returned
				<span class="font-mono">{spf.data.lookup}</span>.
			</p>
		{:else if spf.data && !spf.data.complete}
			<p class="text-muted-foreground text-sm">
				Some include targets did not answer, so the lookup count is unknown. Inspect again to retry.
			</p>
			<div>
				<DetailHeading class="mb-1">Include tree</DetailHeading>
				<SpfIncludeTree node={spf.data.include_tree} />
			</div>
			<DnsFindingsList findings={spf.data.findings} />
		{:else if spf.data}
			{@const s = spf.data}
			{@const trailing = spfTrailing(s)}
			<div class="grid gap-3 sm:grid-cols-3">
				<div class="rounded-md border border-border px-3 py-2">
					<p class="text-muted-foreground text-xs">DNS lookups</p>
					<p class={cn('text-lg font-semibold tabular-nums', s.exceeds_lookup_limit && statusFg.fail)}>
						{spfLookupsLabel(s)}
					</p>
					<p class="text-muted-foreground mt-1 text-xs">
						{#if s.exceeds_lookup_limit}
							Above the RFC 7208 limit.
						{:else}
							Within the RFC 7208 limit.
						{/if}
					</p>
				</div>
				<div class="rounded-md border border-border px-3 py-2">
					<p class="text-muted-foreground text-xs">Void lookups</p>
					<p class="text-lg font-semibold tabular-nums">{spfVoidLookupsLabel(s)}</p>
					<p class="text-muted-foreground mt-1 text-xs">NXDOMAIN or empty answers while walking includes.</p>
				</div>
				<div class="rounded-md border border-border px-3 py-2">
					<p class="text-muted-foreground text-xs">Trailing all</p>
					<p
						class={cn(
							'text-lg font-semibold [overflow-wrap:anywhere]',
							trailing.mono && 'font-mono',
							statusFg[trailing.tone]
						)}
					>
						{trailing.label}
					</p>
					<p class="text-muted-foreground mt-1 text-xs">Applies to every sender not listed.</p>
				</div>
			</div>

			<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
			{#each s.records as record}
				<DnsRawRecord value={record} label="Published SPF record" />
			{/each}

			<div>
				<DetailHeading class="mb-1">Include tree</DetailHeading>
				<p class="text-muted-foreground mb-3 text-xs">
					Own = lookups for that node; subtree = own plus all descendants.
				</p>
				<SpfIncludeTree node={s.include_tree} />
			</div>

			<DnsFindingsList findings={s.findings} />
		{/if}
	</DnsCheckPanel>

	<DnsCheckPanel title="DMARC" {...panel(dmarc)} onRetry={retryDomain} open={false}>
		{#snippet header()}
			{#if dmarc.data}
				{@const d = dmarc.data}
				{@const policy = d.records.length ? d.enforcement : null}
				<span class="font-mono">{d.name}</span>
				{@render verdictBadge(
					worstVerdict(
						policy
							? { label: enforcementLabel(policy, d.parsed?.p), tone: policy.status, mono: true }
							: dmarcRecordStatus(d),
						findingsVerdict(d.findings)
					)
				)}
			{/if}
		{/snippet}
		{#if dmarc.data}
			{@const d = dmarc.data}
			<DmarcDetails check={d} domain={activeDomain} loading={dmarc.loading} onRefresh={retryDomain} />
			<DnsFindingsList findings={d.findings} />
		{/if}
	</DnsCheckPanel>

	<DnsCheckPanel
		title="DKIM"
		{...dkimNeedsSelectors ? { status: 'ready' as const, errorMessage: null } : panel(dkim)}
		onRetry={retryDomain}
		open={false}
	>
		{#snippet header()}
			{#if dkimNeedsSelectors}
				<StatusBadge tone="neutral">Selector needed</StatusBadge>
			{:else if dkim.data}
				{@const k = dkim.data}
				<span class="tabular-nums">
					{plural(k.records.length, 'selector')} checked
				</span>
				{@render verdictBadge(worstVerdict(findingsVerdict(k.findings), selectorVerdict(k.summary)))}
			{/if}
		{/snippet}
		{#if dkimNeedsSelectors}
			<p class="text-muted-foreground text-sm">Enter selectors to check DKIM; they can't be discovered from DNS.</p>
		{:else if dkim.data}
			{@const k = dkim.data}
			<ul class="space-y-3">
				{#each k.records as rec (rec.selector)}
					{@const status = selectorVerdict(rec.verdict)}
					{@const key = dkimKey(rec)}
					<li class="rounded-md border border-border px-3 py-3">
						<div class="mb-2 flex flex-wrap items-center gap-2">
							<span class="font-mono text-sm font-medium">{rec.selector}</span>
							<StatusBadge tone={status.tone} class="text-xs">{status.label}</StatusBadge>
							{#if rec.present}
								<span class={cn('text-xs', statusFg[key.tone])}>{key.label}</span>
							{/if}
						</div>
						<div class="mb-2 flex flex-wrap items-center gap-2 text-xs">
							<span class="text-muted-foreground font-mono">{rec.name}</span>
							<LookupBadge lookup={rec.lookup} />
						</div>
						{#if rec.raw}
							<DnsRawRecord value={rec.raw} label="DKIM record" />
						{/if}
						<DnsFindingsList findings={rec.findings} class="mt-2" />
					</li>
				{/each}
			</ul>
		{/if}
	</DnsCheckPanel>

	<div class="grid gap-4 lg:grid-cols-3">
		<DnsCheckPanel title="MTA-STS" {...panel(mtaSts)} onRetry={retryDomain} open={false}>
			{#snippet header()}
				{#if mtaSts.data}
					{@render verdictBadge(worstVerdict(mtaStsStatus(mtaSts.data), findingsVerdict(mtaSts.data.findings)))}
				{/if}
			{/snippet}
			{#if mtaSts.data}
				{@const m = mtaSts.data}
				{@render nameLine(m.txt_name, m.txt_lookup)}
				{#if m.txt_record}
					<DnsRawRecord value={m.txt_record} label="MTA-STS record" />
				{/if}
				<div class="flex flex-wrap items-center gap-2 text-xs">
					<span class="text-muted-foreground">Policy URL:</span>
					<span class="font-mono break-all">{m.policy_url}</span>
					<span class="text-muted-foreground text-[11px]">
						Fetch: <span class="font-mono">{policyFetchLabel(m.fetch)}</span>
					</span>
				</div>
				{#if m.policy}
					<p class="font-mono text-xs">
						mode={m.policy.mode ?? '—'} max_age={m.policy.max_age ?? '—'} mx={m.policy.mx.join(', ') || '—'}
					</p>
				{/if}
				<DnsFindingsList findings={m.findings} />
			{/if}
		</DnsCheckPanel>

		<DnsCheckPanel title="TLS-RPT" {...panel(tlsRpt)} onRetry={retryDomain} open={false}>
			{#snippet header()}
				{#if tlsRpt.data}
					{@const t = tlsRpt.data}
					{@render verdictBadge(worstVerdict(recordStatus(t.records, t.lookup), findingsVerdict(t.findings)))}
				{/if}
			{/snippet}
			{#if tlsRpt.data}
				{@const t = tlsRpt.data}
				{@render nameLine(t.name, t.lookup)}
				<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
				{#each t.records as record}
					<DnsRawRecord value={record} label="TLS-RPT record" />
				{/each}
				<DnsFindingsList findings={t.findings} />
			{/if}
		</DnsCheckPanel>

		<DnsCheckPanel title="BIMI" {...panel(bimi)} onRetry={retryDomain} open={false}>
			{#snippet header()}
				{#if bimi.data}
					{@const b = bimi.data}
					{@render verdictBadge(worstVerdict(bimiStatus(b), findingsVerdict(b.findings)))}
				{/if}
			{/snippet}
			{#if bimi.data}
				{@const b = bimi.data}
				{@render nameLine(b.name, b.lookup)}
				<!-- eslint-disable-next-line svelte/require-each-key -- a TXT answer is not deduplicated, so the record text can repeat -->
				{#each b.records as record}
					<DnsRawRecord value={record} label="BIMI record" />
				{/each}
				<DnsFindingsList findings={b.findings} />
			{/if}
		</DnsCheckPanel>
	</div>

	<DnsCheckPanel title="PTR (reverse DNS)" status="ready" open={false}>
		{#snippet header()}
			{#if ptr.data}
				{@const count = ptr.data.hostnames.length}
				{#if count > 0}
					<span class="tabular-nums">{plural(count, 'host')}</span>
				{/if}
				{@render verdictBadge(recordStatus(ptr.data.hostnames, ptr.data.lookup))}
			{/if}
		{/snippet}
		<p class="text-muted-foreground text-xs">
			Reverse DNS names for a sending IP. Forward confirmation is shown on the sender's page.
		</p>
		<form class="flex flex-col gap-3 sm:flex-row sm:items-end" onsubmit={onInspectPtr}>
			<div class="min-w-0 flex-1 space-y-1.5">
				<label for="dns-ip" class="text-sm font-medium">IP address</label>
				<Input id="dns-ip" bind:value={ipDraft} placeholder="203.0.113.10" autocomplete="off" spellcheck={false} />
			</div>
			<Button type="submit" variant="secondary" disabled={ipDraft.trim() === ''}>Lookup PTR</Button>
		</form>

		{#if ptr.loading && ptr.data === null}
			<div class="space-y-2" aria-busy="true" aria-label="Looking up PTR">
				<Skeleton class="h-5 w-40" />
				<Skeleton class="w-64" />
			</div>
		{:else if ptr.error}
			<p class="text-status-fail-fg text-sm">{apiErrorMessage(ptr.error)}</p>
		{:else if ptr.data}
			{@const p = ptr.data}
			<div class="flex flex-wrap items-center gap-2 text-xs">
				<span class="font-mono text-sm font-medium">{p.ip}</span>
				<LookupBadge lookup={p.lookup} />
				{#if ptr.loading}
					<span class="text-muted-foreground">Looking up…</span>
				{/if}
			</div>
			{#if p.hostnames.length === 0}
				<p class="text-muted-foreground text-sm">No PTR hostnames returned.</p>
			{:else}
				<ul class="space-y-1">
					{#each p.hostnames as hostname (hostname)}
						<li class="font-mono text-sm">{hostname}</li>
					{/each}
				</ul>
			{/if}
		{/if}
	</DnsCheckPanel>
</div>
