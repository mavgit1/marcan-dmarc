<script lang="ts">
	import { resolve } from '$app/paths';
	import { getAttentionDomains, getDomains } from '$lib/api';
	import type { DomainSummary } from '$lib/types/DomainSummary';
	import type { Page } from '$lib/types/Page';
	import DataTable from '$lib/components/data-table.svelte';
	import type { ColumnDef } from '$lib/components/data-table';
	import IssueBadges from '$lib/components/issue-badges.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { formatCheckedAge, formatDate, formatInt, plural } from '$lib/format';
	import { CHECK_FAILED } from '$lib/dns';
	import { formatRate } from '$lib/health';
	import { filters, scopedHref } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { statusFg, statusFigure } from '$lib/tokens';
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';

	/**
	 * The one domain list: Domains page and organization page. It loads its own rows from
	 * `GET /api/domains`, filtered, sorted and paged on the server (volumes narrowed by the
	 * global time range, coverage all-time). With `attention`, only the domains with an open
	 * issue, from `GET /api/domains/attention` (the dashboard's list, every page): in their
	 * attention rank over the whole scope, the issues column first, no other order; a chip
	 * "Needs attention" says so and clears it (`onClearAttention`).
	 */
	let {
		organizationId,
		showOrganization = true,
		attention = false,
		onClearAttention,
		// eslint-disable-next-line @typescript-eslint/no-unused-vars, no-useless-assignment -- written for the parent's bind:filtered
		filtered = $bindable(false)
	}: {
		/** `null`: every organization. */
		organizationId: number | null;
		/** Hide when every row belongs to the same organization. */
		showOrganization?: boolean;
		/** Only the domains with an open issue, worst first. */
		attention?: boolean;
		/** Clears the attention filter (the chip's ×); without it, no chip. */
		onClearAttention?: () => void;
		/** Out: the filter box narrows the list (the page's figures above it do not follow it). */
		filtered?: boolean;
	} = $props();

	const PAGE_SIZE = 50;
	/** Wait for a pause in typing before filtering. */
	const FILTER_DEBOUNCE_MS = 250;

	let query = $state('');
	let search = $state('');
	$effect(() => {
		const next = query.trim();
		const id = setTimeout(() => (search = next), FILTER_DEBOUNCE_MS);
		return () => clearTimeout(id);
	});

	$effect(() => {
		filtered = search !== '';
	});

	const list = new ServerList('domain');
	/** Another scope shows skeletons; another filter keeps the table (and its sort) on screen. */
	const scopeKey = $derived(`${organizationId}|${filters.since}|${attention}`);
	/** The page goes back to the first whenever the scope or the filter changes. */
	const listKey = $derived(`${scopeKey}|${search}`);

	const domains = resource({
		key: () => scopeKey,
		load: () => {
			const params = {
				organization_id: organizationId,
				since: filters.since,
				q: search,
				limit: PAGE_SIZE,
				offset: list.offsetFor(listKey)
			};
			return attention ? getAttentionDomains(params) : getDomains({ ...params, sort: list.sort });
		},
		// New domains and their first DNS results during an import (throttled to ~15 s).
		reloadOn: () => [search, list.sort, list.offsetFor(listKey), systemHealth.importTick, systemHealth.dnsTick],
		markUpdated: true
	});

	/**
	 * A domain's page, naming the organization the table is narrowed to (`?org=`), so the page
	 * opened from an organization's list starts its trail there.
	 */
	const domainHref = (d: DomainSummary) => scopedHref(`/domains/${d.id}`, { organizationId });

	/** Reporters within the time range; reports over all time. */
	function coverageLabel(d: DomainSummary): string {
		return `${plural(d.reporter_count, 'reporter')} in range · ${plural(d.report_count, 'report')}`;
	}

	const listColumns = $derived.by((): ColumnDef<DomainSummary>[] => [
		{ id: 'domain', header: 'Domain', accessor: (d) => d.domain, sortKey: 'domain' },
		...(showOrganization
			? [
					{
						id: 'organization',
						header: 'Organization',
						accessor: (d: DomainSummary) => d.organization.name,
						sortKey: 'organization_name'
					}
				]
			: []),
		{ id: 'dns', header: 'DNS grade', accessor: (d) => d.dns?.score, sortKey: 'dns_score' },
		{ id: 'policy', header: 'Policy', accessor: (d) => d.policy?.p ?? '', sortKey: 'policy_p' },
		{
			id: 'messages',
			header: 'Messages',
			accessor: (d) => d.total_messages,
			numeric: true,
			sortKey: 'total_messages'
		},
		{
			id: 'compliance',
			header: 'Delivered compliance',
			// Two lines, so the table fits beside the sidebar on a laptop and its header sticks.
			headerClass: 'whitespace-normal',
			accessor: (d) => d.delivered_compliance.pct,
			numeric: true,
			sortKey: 'delivered_pct'
		},
		{
			id: 'failures',
			header: 'Delivered failures',
			// Two lines, so the table fits beside the sidebar on a laptop and its header sticks.
			headerClass: 'whitespace-normal',
			accessor: (d) => d.delivered_failures,
			numeric: true,
			sortKey: 'delivered_failures'
		},
		{
			id: 'last_report',
			header: 'Last report',
			accessor: (d) => d.last_report_at,
			numeric: true,
			sortKey: 'last_report_at'
		}
	]);

	/** The issues column, after the domain: the attention filter's. */
	const issuesColumn: ColumnDef<DomainSummary> = {
		id: 'issues',
		header: 'Issues',
		accessor: (d) => d.assessment.issues.map((i) => i.label).join(', ')
	};

	/** With the attention filter: the issues first, and the server's rank as the only order. */
	const columns = $derived(
		attention
			? [listColumns[0], issuesColumn, ...listColumns.slice(1)].map((c) => ({ ...c, sortKey: undefined }))
			: listColumns
	);
</script>

<div class="mb-3 flex flex-wrap items-center gap-2">
	<div class="relative w-full max-w-md">
		<SearchIcon class="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
		<Input
			type="search"
			class="pl-8"
			placeholder={showOrganization ? 'Filter by domain or organization' : 'Filter by domain'}
			bind:value={query}
			aria-label={showOrganization ? 'Filter domains by domain or organization' : 'Filter domains by name'}
		/>
	</div>
	{#if attention && onClearAttention}
		<span class="bg-muted inline-flex h-7 items-center gap-1 rounded-full border pr-1 pl-3 text-xs font-medium">
			Needs attention
			<button
				type="button"
				class="text-muted-foreground hover:text-foreground hover:bg-background inline-flex size-5 items-center justify-center rounded-full"
				aria-label="Clear the Needs attention filter"
				onclick={onClearAttention}
			>
				<XIcon class="size-3" />
			</button>
		</span>
	{/if}
</div>

{#snippet loaded(page: Page<DomainSummary>)}
	<DataTable
		{columns}
		data={page.items}
		getRowId={(d) => String(d.id)}
		getRowHref={domainHref}
		dense
		server={list.table(listKey, page)}
		emptyHeadline={search ? 'No matching domains' : attention ? 'Nothing needs attention' : 'No domains'}
		emptyDescription={search
			? 'No domain in this scope matches the filter.'
			: attention
				? 'No domain in this scope has an open issue.'
				: undefined}
	>
		{#snippet cell({ column, row })}
			{#if column.id === 'domain'}
				<a href={resolve(domainHref(row) as '/')} class="font-medium hover:underline">{row.domain}</a>
				{#if row.parked}
					<StatusBadge tone="neutral" class="ml-1.5">Parked</StatusBadge>
				{/if}
			{:else if column.id === 'issues'}
				<IssueBadges issues={row.assessment.issues} />
			{:else if column.id === 'organization'}
				<a href={resolve(`/organizations/${row.organization.id}`)} class="hover:underline">
					{row.organization.name}
				</a>
			{:else if column.id === 'dns'}
				{#if row.dns}
					{@const tone = row.assessment.dns}
					<span class="flex flex-col gap-0.5">
						<span class="tabular-nums {statusFigure[tone]}">
							{row.dns.grade}
						</span>
						<span
							class="text-muted-foreground text-[10px]"
							title={row.dns_updating
								? "Marcan's DNS checks changed since this grade; it is being re-checked"
								: undefined}
						>
							{row.dns_updating ? 'updating…' : formatCheckedAge(row.dns.checked_at)}
						</span>
					</span>
				{:else}
					<!-- No stored result: not measured yet, never "missing". -->
					<span class="text-muted-foreground text-xs">
						{row.dns_check_failed ? CHECK_FAILED.label : 'Not checked yet'}
					</span>
				{/if}
			{:else if column.id === 'policy'}
				<PolicyBadge enforcement={row.assessment.enforcement} policy={row.policy?.p} source={row.policy_source} />
			{:else if column.id === 'messages'}
				{formatInt(row.total_messages)}
			{:else if column.id === 'compliance'}
				{@const rate = row.delivered_compliance}
				<span class={statusFigure[rate.status]}>
					{formatRate(rate)}
				</span>
			{:else if column.id === 'failures'}
				<!-- A count, never coloured: the compliance rate next to it carries the verdict. -->
				<span class="tabular-nums {row.delivered_failures > 0 ? 'text-foreground' : 'text-muted-foreground'}">
					{formatInt(row.delivered_failures)}
				</span>
				{#if row.forwarded_failures > 0}
					<span class="text-muted-foreground block text-[10px]" title="Failed in transit, receiver trusted the forward">
						+{formatInt(row.forwarded_failures)} forwarded
					</span>
				{/if}
			{:else if column.id === 'last_report'}
				{#if row.last_report_at == null}
					<span class={statusFg[row.assessment.reporting.status]}>No reports yet</span>
				{:else}
					<span class="flex flex-col items-end gap-0.5">
						<span>{formatDate(row.last_report_at)}</span>
						<!-- May wrap, so the table fits beside the sidebar on a laptop. -->
						<span class="text-muted-foreground text-right text-[10px] whitespace-normal">{coverageLabel(row)}</span>
					</span>
				{/if}
			{/if}
		{/snippet}
	</DataTable>
{/snippet}

{#snippet loading()}
	<DataTable {columns} data={[]} getRowId={(d) => String(d.id)} loading dense />
{/snippet}

<Loaded source={domains} errorTitle="Failed to load domains" data={loaded} {loading} />
