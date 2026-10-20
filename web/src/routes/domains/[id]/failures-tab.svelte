<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getDomainFailures, type DomainFailureFilters } from '$lib/api';
	import type { WithIdentity } from '$lib/types/WithIdentity';
	import type { DomainFailureRow } from '$lib/types/DomainFailureRow';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import type { FailureOutcome } from '$lib/types/FailureOutcome';
	import type { Page } from '$lib/types/Page';
	import { outcomeDisplay } from '$lib/auth-display';
	import {
		FAILURE_KINDS,
		OUTCOME_FILTER_LABEL,
		OUTCOME_FILTER_TITLE,
		failureKindCounts,
		failureKinds,
		failureKindsSummary,
		type FailureKind
	} from '$lib/wording/outcome';
	import AuthChecks from '$lib/components/auth-checks.svelte';
	import DataTable from '$lib/components/data-table.svelte';
	import { defaultCellText, type ColumnDef } from '$lib/components/data-table';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import SenderCell from '$lib/components/sender-cell.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Button, buttonVariants } from '$lib/components/ui/button/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { formatInt, plural } from '$lib/format';
	import { filters, sincePhrase } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import SearchIcon from '@lucide/svelte/icons/search';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import XIcon from '@lucide/svelte/icons/x';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import { cn } from '$lib/utils.js';
	type DomainFailureItem = WithIdentity<DomainFailureRow>;

	const PAGE_SIZE = 50;

	type Row = DomainFailureItem;

	let {
		domainId,
		overview,
		// eslint-disable-next-line @typescript-eslint/no-unused-vars, no-useless-assignment -- written for the parent's bind:exportFilters
		exportFilters = $bindable(null)
	}: {
		domainId: number;
		/** The domain's counts for the page's range: the line above the filters. */
		overview: DomainOverview;
		/** The tab's filters, published for the page's Export dialog. */
		exportFilters?: DomainFailureFilters | null;
	} = $props();

	const OUTCOME_VALUES: readonly string[] = ['all_failures', ...FAILURE_KINDS];

	/**
	 * `?outcome=` (and `?ip=`) deep-link into the tab: the ticked kinds separated by commas
	 * (`all_failures` is the three failure kinds, `all` every kind); without it, the failures.
	 */
	function initialKinds(): FailureKind[] {
		const raw = page.url.searchParams.get('outcome');
		if (raw === 'all') return [...FAILURE_KINDS];
		const values = (raw ?? '')
			.split(',')
			.map((v) => v.trim())
			.filter((v): v is FailureOutcome => OUTCOME_VALUES.includes(v));
		return values.length ? failureKinds(values) : [...FAILURE_KINDS];
	}

	/** What is typed; `sourceIpFilter` follows it after a pause, so typing sends one request. */
	let ipInput = $state(page.url.searchParams.get('ip') || '');
	let sourceIpFilter = $state(page.url.searchParams.get('ip') || '');
	// Every kind is ticked by default; unticking narrows the list.
	let kinds = $state<FailureKind[]>(initialKinds());
	const allKinds = $derived(kinds.length === FAILURE_KINDS.length);

	$effect(() => {
		const typed = ipInput;
		// Clearing applies at once; typing waits for a pause.
		const timer = setTimeout(() => (sourceIpFilter = typed), typed.trim() ? 300 : 0);
		return () => clearTimeout(timer);
	});

	function clearIp() {
		ipInput = '';
		sourceIpFilter = '';
	}

	// Keep both filters in the URL (`?ip=`, `?outcome=`), so a reload or a shared link keeps them.
	$effect(() => {
		const ip = sourceIpFilter.trim();
		const outcome = allKinds ? null : kinds.join(',');
		const url = new URL(page.url);
		const params = url.searchParams;
		if (ip) params.set('ip', ip);
		else params.delete('ip');
		if (outcome) params.set('outcome', outcome);
		else params.delete('outcome');
		// Commas stay readable in a shared link (`?outcome=delivered,dkim_only`).
		const search = url.search.replaceAll('%2C', ',');
		if (search === page.url.search) return;
		void goto(resolve(`${url.pathname}${search}${url.hash}` as '/'), {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	/** The tab's filters; the auth-issues export takes exactly the same ones. */
	const activeFilters = $derived<DomainFailureFilters>({
		since: filters.since,
		// All five kinds is every record: no `outcome` at all.
		outcome: allKinds ? null : kinds,
		source_ip: sourceIpFilter.trim() || null
	});

	$effect(() => {
		exportFilters = activeFilters;
		return () => {
			exportFilters = null;
		};
	});

	const list = new ServerList('-count');
	const scopeKey = $derived(`${domainId}|${filters.since}|${kinds.join(',')}|${sourceIpFilter.trim()}`);

	const failures = resource({
		key: () => scopeKey,
		load: () =>
			getDomainFailures(domainId, {
				...activeFilters,
				sort: list.sort,
				limit: PAGE_SIZE,
				offset: list.offsetFor(scopeKey)
			}),
		reloadOn: () => [list.sort, list.offsetFor(scopeKey)]
	});

	function rowOutcome(r: DomainFailureItem) {
		return outcomeDisplay(r.outcome, {
			verdict: r.verdict,
			disposition: r.disposition,
			overrides: r.overrides
		});
	}

	const columns: ColumnDef<Row>[] = [
		{ id: 'ip', header: 'Source IP', accessor: (r) => r.source_ip, sortKey: 'source_ip' },
		{ id: 'outcome', header: 'Outcome', accessor: (r) => rowOutcome(r).label, sortKey: 'outcome' },
		{ id: 'spf', header: 'SPF', accessor: () => null },
		{ id: 'dkim', header: 'DKIM', accessor: () => null },
		{
			id: 'count',
			header: 'Messages',
			accessor: (r) => r.count,
			numeric: true,
			sortKey: 'count',
			format: (v) => formatInt(Number(v ?? 0))
		},
		{
			id: 'reporter',
			header: 'Reporter',
			accessor: (r) => r.org_name || '—',
			sortKey: 'org_name'
		}
	];

	/**
	 * Messages per kind, from the overview's counts for the domain and range; none while an IP
	 * narrows the list.
	 */
	const showCounts = $derived<Partial<Record<FailureKind, number>>>(
		sourceIpFilter.trim() ? {} : failureKindCounts(overview)
	);
	const showItems = $derived(
		FAILURE_KINDS.map((kind) => {
			const label = OUTCOME_FILTER_LABEL[kind];
			const count = showCounts[kind];
			const ticked = kinds.includes(kind);
			return {
				kind,
				text: count == null ? label : `${label} (${formatInt(count)})`,
				title: OUTCOME_FILTER_TITLE[kind],
				ticked,
				// At least one kind stays ticked.
				locked: ticked && kinds.length === 1
			};
		})
	);
	const showSummary = $derived(failureKindsSummary(kinds));

	function setKind(kind: FailureKind, ticked: boolean) {
		const next = new Set<FailureKind>(kinds);
		if (ticked) next.add(kind);
		else next.delete(kind);
		if (next.size === 0) return;
		kinds = FAILURE_KINDS.filter((k) => next.has(k));
	}

	const emptyHeadline = $derived(
		kinds.length === 1
			? `${OUTCOME_FILTER_LABEL[kinds[0]]}: none in this range`
			: allKinds
				? 'No failures in this range'
				: 'None of these kinds in this range'
	);
</script>

{#if sourceIpFilter.trim()}
	<div class="mb-3 inline-flex items-center gap-2 rounded-md border border-primary/20 bg-primary/5 px-2.5 py-1 text-xs">
		<span class="text-muted-foreground">Filtered by IP:</span>
		<span class="font-mono font-medium text-foreground">{sourceIpFilter.trim()}</span>
		<Button
			type="button"
			variant="ghost"
			size="xs"
			class="h-4 w-4 p-0 text-muted-foreground hover:text-foreground"
			onclick={clearIp}
			aria-label="Clear IP filter"
		>
			<XIcon class="size-3" />
		</Button>
	</div>
{/if}

{#if overview.dmarc_failed_incl_forwarded > 0}
	<p class="text-muted-foreground mb-3 text-sm">
		<span class="text-foreground tabular-nums">{plural(overview.dmarc_failed_incl_forwarded, 'message')}</span>
		failed DMARC {sincePhrase(filters.since)}:
		<span class="tabular-nums">{formatInt(overview.delivered_failures)}</span> delivered,
		<span class="tabular-nums">{formatInt(overview.forwarded_failures)}</span> forwarded,
		<span class="tabular-nums">{formatInt(overview.blocked_messages)}</span> blocked.
	</p>
{/if}

<div class="mb-4 flex flex-wrap items-center gap-3">
	<div class="relative flex items-center">
		<SearchIcon class="text-muted-foreground pointer-events-none absolute left-2 size-3.5" />
		<Input type="text" placeholder="Filter IP…" bind:value={ipInput} class="h-7 w-36 pl-7 pr-6 font-mono text-xs" />
		{#if ipInput}
			<button
				type="button"
				class="text-muted-foreground hover:text-foreground absolute right-1.5 p-0.5 cursor-pointer"
				onclick={clearIp}
				aria-label="Clear IP filter"
			>
				<XIcon class="size-3" />
			</button>
		{/if}
	</div>

	<div class="flex min-w-0 items-center gap-2">
		<span class="text-muted-foreground text-xs font-medium" id="failures-show-label">Show</span>
		<DropdownMenu.Root>
			<DropdownMenu.Trigger
				class={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'min-w-44 justify-between text-xs font-normal')}
				aria-labelledby="failures-show-label failures-show-summary"
			>
				<span class="truncate" id="failures-show-summary">{showSummary}</span>
				<ChevronDownIcon class="text-muted-foreground size-4" />
			</DropdownMenu.Trigger>
			<DropdownMenu.Content align="start" class="min-w-56">
				{#each showItems as item (item.kind)}
					{#if item.kind === 'dkim_only'}
						<DropdownMenu.Separator />
					{/if}
					<DropdownMenu.CheckboxItem
						checked={item.ticked}
						onCheckedChange={(ticked) => setKind(item.kind, ticked)}
						disabled={item.locked}
						closeOnSelect={false}
						title={item.title}
						class="text-xs"
					>
						<span class="tabular-nums">{item.text}</span>
					</DropdownMenu.CheckboxItem>
				{/each}
			</DropdownMenu.Content>
		</DropdownMenu.Root>
	</div>
</div>

{#snippet table(result: Page<Row>)}
	<DataTable
		{columns}
		data={result.items}
		getRowId={(r, i) =>
			`${r.source_ip}|${r.outcome}|${r.disposition ?? 'none'}|${r.org_name ?? ''}|${JSON.stringify([r.spf_auth, r.dkim_auth, r.overrides])}|${i}`}
		getRowHref={(r) => `/senders/${encodeURIComponent(r.source_ip)}`}
		dense
		server={list.table(scopeKey, result)}
		{emptyHeadline}
	>
		{#snippet cell({ column, row })}
			{#if column.id === 'ip'}
				<SenderCell ip={row.source_ip} identity={row.identity} />
			{:else if column.id === 'outcome'}
				{@const outcome = rowOutcome(row)}
				<span title={outcome.title}>
					<StatusBadge tone={outcome.tone} class="whitespace-nowrap">{outcome.label}</StatusBadge>
				</span>
			{:else if column.id === 'spf'}
				<AuthChecks kind="spf" checks={row.spf_auth} />
			{:else if column.id === 'dkim'}
				<AuthChecks kind="dkim" checks={row.dkim_auth} />
			{:else if column.id === 'reporter'}
				<span class="inline-block max-w-[160px] truncate align-middle text-xs" title={row.org_name || '—'}>
					{row.org_name || '—'}
				</span>
			{:else}
				{defaultCellText(column, row)}
			{/if}
		{/snippet}
	</DataTable>
{/snippet}

{#snippet loaded(result: Page<Row>)}
	{#if result.total === 0}
		<EmptyState
			icon={TriangleAlertIcon}
			headline={emptyHeadline}
			description={allKinds
				? 'Either mail is authenticating, or there are no aggregate reports yet for this domain.'
				: 'Nothing matches the kinds ticked under Show. Tick more, or try a longer time range.'}
		/>
	{:else}
		{@render table(result)}
	{/if}
{/snippet}

{#snippet loading()}
	<DataTable {columns} data={[]} getRowId={(r, i) => `${r.source_ip}__${i}`} loading dense />
{/snippet}

<Loaded source={failures} errorTitle="Failures failed to load" data={loaded} {loading} />
