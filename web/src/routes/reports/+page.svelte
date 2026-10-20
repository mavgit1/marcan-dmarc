<script lang="ts">
	import { resolve } from '$app/paths';
	import { getReports } from '$lib/api';
	import type { Page } from '$lib/types/Page';
	import type { ReportSummary } from '$lib/types/ReportSummary';
	import DataTable from '$lib/components/data-table.svelte';
	import { defaultCellText, type ColumnDef } from '$lib/components/data-table';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { formatDateRange, formatInt } from '$lib/format';
	import { formatRate } from '$lib/health';
	import { filters } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import { STATUS_WORD, statusFg } from '$lib/tokens';
	import FileBarChartIcon from '@lucide/svelte/icons/file-bar-chart';
	import SearchIcon from '@lucide/svelte/icons/search';

	const PAGE_SIZE = 50;
	/** Wait for a pause in typing before searching. */
	const SEARCH_DEBOUNCE_MS = 250;

	let query = $state('');
	let search = $state('');
	$effect(() => {
		const next = query.trim();
		const id = setTimeout(() => (search = next), SEARCH_DEBOUNCE_MS);
		return () => clearTimeout(id);
	});

	const list = new ServerList('-date_begin');
	const scopeKey = $derived(`${filters.organizationId ?? ''}:${filters.domainId ?? ''}:${filters.since}:${search}`);

	const reports = resource({
		key: () => scopeKey,
		load: () =>
			getReports({
				organization_id: filters.organizationId,
				domain_id: filters.domainId,
				since: filters.since,
				q: search,
				sort: list.sort,
				limit: PAGE_SIZE,
				offset: list.offsetFor(scopeKey)
			}),
		reloadOn: () => [list.sort, list.offsetFor(scopeKey)],
		markUpdated: true
	});

	const columns: ColumnDef<ReportSummary>[] = [
		{ id: 'domain', header: 'Domain', accessor: (r) => r.domain, sortKey: 'domain' },
		{ id: 'org', header: 'Reporter', accessor: (r) => r.org_name, sortKey: 'org_name' },
		{
			id: 'range',
			header: 'Date range',
			accessor: (r) => r.date_begin,
			sortKey: 'date_begin',
			numeric: true,
			align: 'left',
			format: (_v, r) => formatDateRange(r.date_begin, r.date_end)
		},
		{
			id: 'messages',
			header: 'Messages',
			accessor: (r) => r.total_messages,
			numeric: true,
			sortKey: 'total_messages',
			format: (v) => formatInt(Number(v ?? 0))
		},
		{
			id: 'delivered',
			header: 'Delivered compliance',
			accessor: (r) => r.delivered_compliance.pct,
			numeric: true,
			sortKey: 'delivered_pct'
		},
		{
			id: 'blocked',
			header: 'Blocked by policy',
			accessor: (r) => r.blocked_messages,
			numeric: true,
			sortKey: 'blocked',
			format: (v) => formatInt(Number(v ?? 0))
		},
		{ id: 'policy', header: 'Policy', accessor: (r) => r.policy_p ?? '', sortKey: 'policy_p' }
	];
</script>

<PageHeader title="Reports" />

<div class="mb-3 relative max-w-md">
	<SearchIcon class="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
	<Input
		type="search"
		class="pl-8"
		placeholder="Search by domain or reporter"
		bind:value={query}
		aria-label="Search reports by domain or reporter"
	/>
</div>

{#snippet loaded(page: Page<ReportSummary>)}
	{#if page.total === 0 && !search}
		<EmptyState
			icon={FileBarChartIcon}
			headline="No reports in this scope"
			description="Adjust the organization, domain, or time range in the top bar."
		/>
	{:else}
		{@render table(page)}
	{/if}
{/snippet}

{#snippet loading()}
	<DataTable {columns} data={[]} getRowId={(r) => String(r.id)} loading dense />
{/snippet}

{#snippet table(page: Page<ReportSummary>)}
	<DataTable
		{columns}
		data={page.items}
		getRowId={(r) => String(r.id)}
		getRowHref={(r) => `/reports/${r.id}`}
		dense
		server={list.table(scopeKey, page)}
		emptyHeadline="No matching reports"
		emptyDescription="No report in this scope matches the search."
	>
		{#snippet cell({ column, row })}
			{#if column.id === 'domain'}
				<a href={resolve(`/domains/${row.domain_id}`)} class="hover:underline" onclick={(e) => e.stopPropagation()}>
					{row.domain}
				</a>
			{:else if column.id === 'policy'}
				<div class="flex items-center gap-1.5 whitespace-nowrap">
					{#if row.policy_p}
						<PolicyBadge enforcement={row.enforcement} policy={row.policy_p} />
					{:else}
						<span class="text-muted-foreground text-xs">—</span>
					{/if}
					{#if row.policy_sp && row.policy_sp !== row.policy_p}
						<span class="text-muted-foreground font-mono text-xs">sp={row.policy_sp}</span>
					{/if}
					{#if row.policy_pct != null && row.policy_pct < 100}
						<span class="text-muted-foreground font-mono text-xs tabular-nums">pct={row.policy_pct}%</span>
					{/if}
				</div>
			{:else if column.id === 'delivered'}
				{@const rate = row.delivered_compliance}
				{@const tone = rate.status}
				<span
					class={statusFg[tone]}
					title="Share of delivered mail that authenticated. Mail blocked by policy is excluded."
				>
					{formatRate(rate)}
				</span>
				<span class="sr-only">{STATUS_WORD[tone]}</span>
			{:else if column.id === 'blocked'}
				{@const blocked = row.blocked_messages}
				<span
					class={blocked > 0 ? statusFg.info : 'text-muted-foreground'}
					title="Messages the receiver quarantined or rejected -- DMARC working as designed."
				>
					{formatInt(blocked)}
				</span>
			{:else}
				{defaultCellText(column, row)}
			{/if}
		{/snippet}
	</DataTable>
{/snippet}

<Loaded source={reports} errorTitle="Reports failed to load" data={loaded} {loading} />
