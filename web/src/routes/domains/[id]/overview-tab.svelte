<script lang="ts">
	import { page } from '$app/state';
	import { getDnsCheck, getTimeseries } from '$lib/api';
	import { apiErrorMessage } from '$lib/http';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import DomainScorecard from '$lib/components/domain-scorecard.svelte';
	import VolumeChart from '$lib/components/volume-chart.svelte';
	import { seriesFromTimeseries, type DailyPoint } from '$lib/series';
	import { filters } from '$lib/state/filters.svelte';
	import ReportingDomain from '$lib/components/reporting-setup/reporting-domain.svelte';
	import { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import { isReporting, receivesReports, REPORTING_STATUS_LABEL } from '$lib/report-address';
	import { resource } from '$lib/state/resource.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { untrack } from 'svelte';
	import { dnsCheckedAt } from './dns-sync';
	import MailWarningIcon from '@lucide/svelte/icons/mail-warning';
	import RecentChanges from '../../alerts/recent-changes.svelte';

	/** Figures (including delivered failures) come with the overview, for the page's window. */
	let { overview }: { overview: DomainOverview } = $props();

	const focusParam = $derived(page.url.searchParams.get('focus'));

	/**
	 * `null` while loading; the chart shows its empty state when the request fails. A refresh
	 * keeps the chart until the new series arrives; another domain or range starts empty.
	 */
	// The id, not the object: each refresh brings a new `overview`, and the tick already reloads.
	const seriesDomainId = $derived(overview.id);
	const timeseries = resource({
		key: () => `${seriesDomainId}|${filters.since}`,
		load: () =>
			getTimeseries({ domain_id: seriesDomainId, since: filters.since, bucket: 'day' }).then((res) =>
				seriesFromTimeseries(res.items)
			)
	});
	const series = $derived<DailyPoint[] | null>(timeseries.data ?? (timeseries.error ? [] : null));

	// DNS has its own freshness (the server re-checks stale results); a report refresh
	// does not touch it, so only another domain fetches again, or the Re-check button.
	const dnsDomain = $derived(overview.domain);
	const dns = resource({
		key: () => dnsDomain,
		load: (opts?: { recheck?: boolean }) => getDnsCheck(dnsDomain, { refresh: !!opts?.recheck }),
		refetchOnRefresh: false
	});
	const currentDns = $derived(dns.data);
	const dnsError = $derived(dns.error === null ? null : apiErrorMessage(dns.error));

	/**
	 * One page shows one DNS result. The header's policy, the readiness line and the
	 * Enforcement tab come with the overview (the stored check as the page loaded); the
	 * scorecard has its own answer, which may be a newer check (this page's first or a
	 * re-check, or one the server ran meanwhile), while a report refresh may bring the
	 * overview a newer stored check than the scorecard's. Whichever is older loads again, once
	 * per pair of results; an answer without a complete result changes nothing.
	 */
	let syncedFor: string | null = null;
	$effect(() => {
		if (!currentDns || currentDns.status === 'failed') return;
		const shown = dnsCheckedAt(currentDns.checked_at);
		const stored = overview.dns?.checked_at ?? null;
		if (shown == null || shown === stored) return;
		const pair = `${overview.id}|${shown}|${stored}`;
		if (syncedFor === pair) return;
		syncedFor = pair;
		untrack(() => (stored == null || shown > stored ? refresh.refresh() : dns.reload()));
	});

	/** The Re-check button: again, bypassing the server's caches. */
	function refreshDns() {
		dns.reload({ recheck: true });
	}

	/** Whether an account receives reports, for the setup section; a failed load hides it. */
	$effect(() => {
		settings.load().catch(() => {});
	});

	/**
	 * No recent report: this domain's DMARC record as the reporting setup needs it, its status
	 * decided by the server (the address domains' own part is the setup's, under /setup). The
	 * server reads the record from the stored DNS check, so it is asked once the scorecard's
	 * answer is in (a first check is then stored, not run twice) and again whenever that answer
	 * changes (Re-check); it has no re-check of its own.
	 */
	const wantSetup = $derived(receivesReports(settings.current) && !isReporting(overview));
	const dnsAnswered = $derived(currentDns != null || dns.error != null);
	const setup = new ReportingSetupState({
		domainId: () => overview.id,
		enabled: () => wantSetup && dnsAnswered,
		reloadOn: () => currentDns?.checked_at
	});
	const setupRow = $derived(wantSetup ? (setup.data?.domains[0] ?? null) : null);
</script>

<div class="space-y-3">
	{#if setupRow && setup.data && setupRow.state !== 'reporting'}
		<section class={['space-y-3 rounded-lg border p-4', !setupRow.set_up && 'border-status-warn-border']}>
			<h2 class={['flex items-center gap-2 text-sm font-medium', !setupRow.set_up && 'text-status-warn-fg']}>
				<MailWarningIcon class="size-4" />
				{setupRow.set_up ? REPORTING_STATUS_LABEL[setupRow.state] : "Reports don't reach Marcan yet"}
			</h2>
			<ul class="divide-border divide-y rounded-lg border text-xs leading-relaxed">
				<ReportingDomain row={setupRow} {setup} defaultAddress={setup.data.default_address} recheck={false} />
			</ul>
		</section>
	{/if}

	{#if overview.last_report_at != null}
		<VolumeChart series={series ?? []} loading={series == null} outcomes={{ mail: overview, total: true }} />
	{/if}

	<DomainScorecard
		{overview}
		dns={currentDns}
		dnsLoading={dns.loading}
		{dnsError}
		initialFocus={focusParam}
		onRefresh={refreshDns}
	/>

	<RecentChanges domainId={overview.id} organizationId={overview.organization.id} />
</div>
