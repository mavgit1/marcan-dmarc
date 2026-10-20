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
	import { isReporting, REPORTING_STATUS_LABEL } from '$lib/report-address';
	import { resource } from '$lib/state/resource.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { untrack } from 'svelte';
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
	 * A domain shown before its first DNS check (policy "Not checked yet") gets its first stored
	 * result from this page's check: reload the overview once so the header's policy follows.
	 */
	let reloadedFor: number | null = null;
	$effect(() => {
		const pending = overview.policy_source === 'not_checked' || overview.policy_source === 'check_failed';
		if (!pending || !currentDns || currentDns.status === 'failed' || reloadedFor === overview.id) return;
		reloadedFor = overview.id;
		untrack(() => refresh.refresh());
	});

	/** The Re-check button: again, bypassing the server's caches. */
	function refreshDns() {
		dns.reload({ recheck: true });
	}

	/** The report address and mode, for the setup section; a failed load hides it. */
	$effect(() => {
		settings.load().catch(() => {});
	});
	const reporting = $derived(settings.current?.reporting ?? null);

	/**
	 * No recent report: this domain's DMARC record as the reporting setup needs it, its status
	 * decided by the server (the mailbox domain's own part is the setup's, under /setup).
	 */
	const wantSetup = $derived(!!reporting?.report_address.trim() && !isReporting(overview));
	const setup = new ReportingSetupState({ domainId: () => overview.id, enabled: () => wantSetup });
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
				<ReportingDomain
					row={setupRow}
					{setup}
					mode={setup.data.reporting_mode}
					mailboxDomain={setup.data.mailbox_domain}
				/>
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
