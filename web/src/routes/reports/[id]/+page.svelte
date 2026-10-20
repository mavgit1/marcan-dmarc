<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { getReport } from '$lib/api';
	import { isNotFound } from '$lib/http';
	import type { ReportRecord } from '$lib/types/ReportRecord';
	import type { ReportDetailResponse } from '$lib/types/ReportDetailResponse';
	import { outcomeDisplay } from '$lib/auth-display';
	import AuthChecks from '$lib/components/auth-checks.svelte';
	import DataTable from '$lib/components/data-table.svelte';
	import { defaultCellText, type ColumnDef } from '$lib/components/data-table';
	import HealthSummary from '$lib/components/health-summary.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import NotFoundState from '$lib/components/not-found-state.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StatCard from '$lib/components/stat-card.svelte';
	import SenderCell from '$lib/components/sender-cell.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { formatDateRange, formatInt } from '$lib/format';
	import { pageMeta } from '$lib/state/page-meta.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { formatRate } from '$lib/health';
	import { statusRow } from '$lib/tokens';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { isAuthFailure, sortRecordsForensics } from '../record-utils';

	type Row = ReportRecord;

	/** The legend's total: the report is the window, not the header's range. */
	const REPORT_TOTAL = 'in this report';

	let failuresOnly = $state(false);

	const reportId = $derived(Number(page.params.id));
	const validId = $derived(Number.isInteger(reportId) && reportId > 0);

	const detail = resource({
		key: () => String(reportId),
		load: () => (validId ? getReport(reportId) : null),
		markUpdated: true
	});
	const notFound = $derived(!validId || isNotFound(detail.error));

	$effect(() => {
		const r = detail.data;
		pageMeta.entityName = r && !notFound ? `${r.domain} · ${r.org_name}` : null;
	});
	$effect(() => () => {
		pageMeta.entityName = null;
	});

	function rowOutcome(r: ReportRecord) {
		return outcomeDisplay(r.outcome, {
			verdict: r.verdict,
			disposition: r.disposition,
			overrides: r.overrides
		});
	}

	/** Same columns as the domain's Failures tab; the reporter is the page's own. */
	const recordColumns: ColumnDef<Row>[] = [
		{ id: 'ip', header: 'Source IP', accessor: (r) => r.source_ip, sortable: true },
		{
			id: 'outcome',
			header: 'Outcome',
			accessor: (r) => r.outcome_rank,
			sortable: true
		},
		{ id: 'spf', header: 'SPF', accessor: () => null },
		{ id: 'dkim', header: 'DKIM', accessor: () => null },
		{
			id: 'count',
			header: 'Messages',
			accessor: (r) => r.count,
			numeric: true,
			sortable: true,
			format: (v) => formatInt(Number(v ?? 0))
		}
	];
</script>

{#if notFound}
	<PageHeader title="Report" />
	<NotFoundState
		headline="Report not found"
		description="No report matches this identifier."
		backHref={resolve('/reports')}
		backLabel="Back to reports"
	/>
{:else}
	<Loaded source={detail} errorTitle="Report failed to load" {header} data={body} {loading} {error} />
{/if}

{#snippet error(err: unknown)}
	<PageHeader title="Report" />
	<ErrorState title="Report failed to load" error={err} onRetry={() => refresh.refresh()} />
{/snippet}

{#snippet loading()}
	<PageHeader title="Report" />
	<div class="space-y-4">
		<HealthSummary loading mail={null} outcomes={{ mail: null, total: REPORT_TOTAL }} />
		<div class="grid gap-3 sm:grid-cols-3">
			<StatCard label="Delivered compliance" loading />
		</div>
		<Skeleton variant="block" class="h-48" />
	</div>
{/snippet}

{#snippet header(report: ReportDetailResponse)}
	<PageHeader
		title={report.domain}
		description={`Reporter ${report.org_name} · ${formatDateRange(report.date_begin, report.date_end)}`}
	>
		{#snippet titleAddon()}
			<div class="flex items-center gap-1.5 whitespace-nowrap">
				{#if report.policy_p}
					<PolicyBadge enforcement={report.enforcement} policy={report.policy_p} />
				{/if}
				{#if report.subdomain_enforcement && report.policy_sp !== report.policy_p}
					<span class="text-muted-foreground text-xs">sp</span>
					<PolicyBadge enforcement={report.subdomain_enforcement} policy={report.policy_sp} />
				{/if}
				{#if report.policy_pct != null && report.policy_pct < 100}
					<span class="text-muted-foreground font-mono text-xs tabular-nums">pct={report.policy_pct}%</span>
				{/if}
			</div>
		{/snippet}
	</PageHeader>
{/snippet}

{#snippet body(report: ReportDetailResponse)}
	{@const ranked = sortRecordsForensics(report.records)}
	{@const failureCount = ranked.filter(isAuthFailure).length}
	{@const rows = failuresOnly ? ranked.filter(isAuthFailure) : ranked}

	<p class="text-muted-foreground mb-6 text-xs">
		{report.email} · <span class="font-mono">{report.report_id}</span>
	</p>

	<HealthSummary class="mb-3" mail={report} outcomes={{ mail: report, total: REPORT_TOTAL }} />

	<div class="mb-6 grid gap-3 sm:grid-cols-3">
		<StatCard
			label="Delivered compliance"
			value={formatRate(report.delivered_compliance)}
			hint={`${formatInt(report.compliant_messages)} of ${formatInt(report.delivered_messages)} delivered messages passed DMARC`}
			icon={ShieldCheckIcon}
		/>
	</div>

	<section class="space-y-3">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<h2 class="text-sm font-medium">Records</h2>
				<p class="text-muted-foreground text-xs">Worst outcome first unless you sort a column.</p>
			</div>
			<Button
				variant={failuresOnly ? 'default' : 'outline'}
				size="sm"
				aria-pressed={failuresOnly}
				onclick={() => (failuresOnly = !failuresOnly)}
			>
				Failures only
				<span class="tabular-nums">({formatInt(failureCount)})</span>
			</Button>
		</div>

		<DataTable
			columns={recordColumns}
			data={rows}
			getRowId={(r) => String(r.id)}
			rowClass={(r) => (r.outcome === 'delivered' ? statusRow.fail : undefined)}
			dense
			paginate={false}
			emptyHeadline={failuresOnly ? 'No failing records' : 'No records'}
			emptyDescription={failuresOnly
				? 'Every row in this report passed DMARC (an aligned SPF or DKIM pass).'
				: 'This report has no record rows.'}
		>
			{#snippet cell({ column, row })}
				{#if column.id === 'ip'}
					<SenderCell
						ip={row.source_ip}
						identity={row.identity}
						href={resolve(`/senders/${encodeURIComponent(row.source_ip)}`)}
					>
						{#if row.header_from && row.header_from.toLowerCase() !== report.domain.toLowerCase()}
							<span class="text-muted-foreground text-xs" title="Header From domain of these messages">
								From <span class="font-mono">{row.header_from}</span>
							</span>
						{/if}
					</SenderCell>
				{:else if column.id === 'outcome'}
					{@const outcome = rowOutcome(row)}
					<span title={outcome.title}>
						<StatusBadge tone={outcome.tone} class="whitespace-nowrap">{outcome.label}</StatusBadge>
					</span>
				{:else if column.id === 'spf'}
					<AuthChecks kind="spf" checks={row.spf_auth} />
				{:else if column.id === 'dkim'}
					<AuthChecks kind="dkim" checks={row.dkim_auth} />
				{:else}
					{defaultCellText(column, row)}
				{/if}
			{/snippet}
		</DataTable>
	</section>
{/snippet}
