<script lang="ts">
	import { getSenders } from '#lib/api.ts';
	import type { Page } from '#lib/types/Page.ts';
	import type { WithIdentity } from '#lib/types/WithIdentity.ts';
	import type { SenderSummary } from '#lib/types/SenderSummary.ts';
	import { ALIGNED_PASS_LABEL } from '#lib/auth-display.ts';
	import SenderCell from '#lib/components/sender-cell.svelte';
	import DataTable from '#lib/components/data-table.svelte';
	import { defaultCellText, type ColumnDef } from '#lib/components/data-table.ts';
	import EmptyState from '#lib/components/empty-state.svelte';
	import Loaded from '#lib/components/loaded.svelte';
	import SenderName from '#lib/components/sender-name.svelte';
	import { formatInt, formatPct, plural } from '#lib/format.ts';
	import { displayName } from '#lib/identity.ts';
	import { filters } from '#lib/state/filters.svelte.ts';
	import { resource } from '#lib/state/resource.svelte.ts';
	import { ServerList } from '#lib/state/server-list.svelte.ts';
	import { rowTint, statusFg } from '#lib/tokens.ts';
	import { Input } from '#lib/components/ui/input/index.ts';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ServerIcon from '@lucide/svelte/icons/server';
	type Sender = WithIdentity<SenderSummary>;

	/**
	 * The one sender list, server-sorted and paged. Without `domainId` it covers the global
	 * scope (organization, domain, time range) and shows failed messages and how many domains
	 * an IP sends as; with `domainId` it covers that domain (and the time range) and shows its
	 * SPF / DKIM passes and the IP's other domains. A search box narrows either by IP or sender
	 * name, on the server (the names are resolved there).
	 */
	let { domainId }: { domainId?: number } = $props();

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

	const list = new ServerList('-total_messages');
	/** Another scope shows skeletons; another search keeps the table (and its sort) on screen. */
	const scopeKey = $derived(
		domainId !== undefined
			? `domain:${domainId}|${filters.since}`
			: `${filters.organizationId}|${filters.domainId}|${filters.since}`
	);
	/** The page goes back to the first whenever the scope or the search changes. */
	const listKey = $derived(`${scopeKey}|${search}`);

	const senders = resource({
		key: () => scopeKey,
		load: () =>
			getSenders({
				...(domainId !== undefined
					? { domain_id: domainId }
					: { organization_id: filters.organizationId, domain_id: filters.domainId }),
				since: filters.since,
				q: search,
				sort: list.sort,
				limit: PAGE_SIZE,
				offset: list.offsetFor(listKey)
			}),
		reloadOn: () => [search, list.sort, list.offsetFor(listKey)],
		markUpdated: true
	});

	const intColumn = (
		id: string,
		header: string,
		accessor: (r: Sender) => number,
		sortKey?: string
	): ColumnDef<Sender> => ({
		id,
		header,
		accessor,
		numeric: true,
		sortKey,
		format: (v) => formatInt(Number(v ?? 0))
	});

	const columns = $derived.by((): ColumnDef<Sender>[] => [
		{
			id: 'sender',
			header: 'Sender',
			accessor: (r) => displayName(r.identity),
			// A long name ("Google (Including Gmail and Google Workspace)") wraps, so the table fits a laptop window.
			class: 'min-w-36 whitespace-normal'
		},
		{ id: 'ip', header: 'Source IP', accessor: (r) => r.source_ip, sortKey: 'source_ip' },
		intColumn('messages', 'Messages', (r) => r.total_messages, 'total_messages'),
		...(domainId === undefined ? [intColumn('failed', 'Failed', (r) => r.failed_messages, 'failed_messages')] : []),
		{
			id: 'compliance',
			header: 'DMARC pass',
			accessor: (r) => r.compliance_pct,
			numeric: true,
			sortKey: 'compliance_pct',
			format: (v) => formatPct(Number(v ?? 0))
		},
		...(domainId === undefined
			? [
					// All time on purpose: an IP sending as many domains is shared or spoofing.
					intColumn('domains', 'Domains (all time)', (r) => r.domain_count)
				]
			: [
					// Two lines, so the table fits beside the sidebar on a laptop and its header sticks.
					{
						...intColumn('spf_pass', ALIGNED_PASS_LABEL.spf, (r) => r.spf_pass_messages, 'spf_pass_messages'),
						headerClass: 'whitespace-normal'
					},
					{
						...intColumn('dkim_pass', ALIGNED_PASS_LABEL.dkim, (r) => r.dkim_pass_messages, 'dkim_pass_messages'),
						headerClass: 'whitespace-normal'
					},
					intColumn('other_domains', 'Other domains', (r) => r.domain_count - 1)
				])
	]);

	const empty = $derived(
		domainId === undefined
			? {
					headline: 'No senders in this scope',
					description: 'Adjust the organization, domain, or time range in the top bar.'
				}
			: {
					headline: 'No senders in this range',
					description: 'No aggregate reports name a source IP for this domain yet.'
				}
	);
</script>

<div class="relative mb-3 max-w-md">
	<SearchIcon class="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2" />
	<Input
		type="search"
		class="pl-8"
		placeholder="Search by IP or sender name"
		bind:value={query}
		aria-label="Search senders by IP or sender name"
	/>
</div>

{#snippet loaded(page: Page<Sender>)}
	{#if page.total === 0 && !search}
		<EmptyState icon={ServerIcon} headline={empty.headline} description={empty.description} />
	{:else}
		<DataTable
			{columns}
			data={page.items}
			getRowId={(r) => r.source_ip}
			getRowHref={(r) => `/senders/${encodeURIComponent(r.source_ip)}`}
			rowClass={(r) => rowTint(r.compliance_status)}
			dense
			server={list.table(listKey, page)}
			emptyHeadline={search ? 'No matching senders' : empty.headline}
			emptyDescription={search ? 'No sender matches your search.' : empty.description}
		>
			{#snippet cell({ column, row })}
				{#if column.id === 'sender'}
					<SenderName identity={row.identity} />
				{:else if column.id === 'ip'}
					<SenderCell ip={row.source_ip} />
				{:else if column.id === 'failed'}
					<!-- Delivered failures and blocked; a trusted forward is counted apart. -->
					{formatInt(row.failed_messages)}
					{#if row.forwarded_failures > 0}
						<span
							class="text-muted-foreground block text-[10px]"
							title="Failed in transit, receiver trusted the forward"
						>
							+{formatInt(row.forwarded_failures)} forwarded
						</span>
					{/if}
				{:else if column.id === 'compliance'}
					<span class={statusFg[row.compliance_status]}>{formatPct(row.compliance_pct)}</span>
				{:else if column.id === 'other_domains'}
					{@const others = row.domain_count - 1}
					{#if others > 0}
						<span title="Also sends as other domains (all time)">{plural(others, 'other')}</span>
					{:else}
						<span class="text-muted-foreground">Unique to this domain</span>
					{/if}
				{:else}
					{defaultCellText(column, row)}
				{/if}
			{/snippet}
		</DataTable>
	{/if}
{/snippet}

{#snippet loading()}
	<DataTable {columns} data={[]} getRowId={(r) => r.source_ip} loading dense />
{/snippet}

<Loaded source={senders} errorTitle="Senders failed to load" data={loaded} {loading} />
