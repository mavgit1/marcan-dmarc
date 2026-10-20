<script lang="ts">
	import { resolve } from '$app/paths';
	import type { DomainSummary } from '$lib/types/DomainSummary';
	import DataTable from '$lib/components/data-table.svelte';
	import type { ColumnDef } from '$lib/components/data-table';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import type { IssueKind } from '$lib/types/IssueKind';
	import { meta } from '$lib/state/meta.svelte';
	import { formatInt, plural } from '$lib/format';
	import type { StatusTone } from '$lib/tokens';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { WORST_FIRST_SORT } from '$lib/components/domains-table.svelte';

	type AttentionRow = {
		domainId: number;
		domain: string;
		parked: boolean;
		messages: number;
		issues: { kind: IssueKind; label: string; tone: StatusTone }[];
		rank: number;
	};

	let {
		domains,
		total
	}: {
		/**
		 * The first domains with an open issue (`GET /api/domains/attention`), in the server's
		 * `attention_rank` over the whole scope, never one page of the domain list nor by volume.
		 */
		domains: DomainSummary[];
		/** How many domains have an open issue in all. */
		total: number;
	} = $props();

	/** The rest: the Domains page, lowest DNS score first (worst first among its sorts). */
	const more = $derived(Math.max(total - domains.length, 0));
	const moreHref = $derived(resolve(`/domains?sort=${WORST_FIRST_SORT}` as '/domains'));

	/** What each issue means, the badge's tooltip: the server's catalogue (`meta.issue_catalogue`). */
	const meaning = $derived(new Map(meta.current?.issue_catalogue.map((i) => [i.kind, i.meaning]) ?? []));
	$effect(() => {
		meta.load().catch(() => {});
	});

	/**
	 * Every open issue (the server's `assessment.issues`, worst first, each with its badge), one
	 * row per domain, so nothing is dropped and nothing repeats. Rows in the server's
	 * `attention_rank`.
	 */
	function buildRows(list: DomainSummary[]): AttentionRow[] {
		const rows: AttentionRow[] = [];

		for (const d of list) {
			if (d.attention_rank == null) continue;
			rows.push({
				domainId: d.id,
				domain: d.domain,
				parked: d.parked,
				messages: d.total_messages,
				issues: d.assessment.issues.map((i) => ({ kind: i.kind, label: i.label, tone: i.status })),
				rank: d.attention_rank
			});
		}

		return rows.sort((a, b) => a.rank - b.rank);
	}

	const rows = $derived(buildRows(domains));

	const columns: ColumnDef<AttentionRow>[] = [
		{ id: 'domain', header: 'Domain', accessor: (r) => r.domain },
		{ id: 'issues', header: 'Issues', accessor: (r) => r.issues.map((i) => i.label).join(', ') },
		{ id: 'messages', header: 'Messages', accessor: (r) => r.messages, numeric: true },
		{ id: 'open', header: '', accessor: () => '' }
	];
</script>

{#if rows.length > 0}
	<section class="space-y-3" aria-label="Needs attention">
		<div class="flex items-baseline justify-between gap-2">
			<h2 class="text-sm font-medium">Needs attention</h2>
			<p class="text-muted-foreground text-xs">
				{plural(total, 'domain')}
			</p>
		</div>
		<DataTable
			{columns}
			data={rows}
			getRowId={(r) => String(r.domainId)}
			getRowHref={(r) => `/domains/${r.domainId}`}
			dense
			paginate={false}
		>
			{#snippet cell({ column, row })}
				{#if column.id === 'domain'}
					<span class="font-medium">{row.domain}</span>
					{#if row.parked}
						<StatusBadge tone="neutral" class="ml-1.5">Parked</StatusBadge>
					{/if}
				{:else if column.id === 'issues'}
					<div class="flex flex-wrap gap-1.5">
						{#each row.issues as issue (issue.kind)}
							<span title={meaning.get(issue.kind)}>
								<StatusBadge tone={issue.tone}>{issue.label}</StatusBadge>
							</span>
						{/each}
					</div>
				{:else if column.id === 'messages'}
					<span class="tabular-nums">{formatInt(row.messages)}</span>
				{:else}
					<a
						href={resolve(`/domains/${row.domainId}`)}
						class="text-muted-foreground hover:text-foreground inline-flex items-center gap-0.5 text-xs whitespace-nowrap"
					>
						Open
						<ChevronRightIcon class="size-3" />
					</a>
				{/if}
			{/snippet}
		</DataTable>
		{#if more > 0}
			<a href={moreHref} class="text-muted-foreground hover:text-foreground inline-flex items-center gap-0.5 text-xs">
				{plural(more, 'more needs attention', 'more need attention')}
				<ChevronRightIcon class="size-3" />
			</a>
		{/if}
	</section>
{/if}
