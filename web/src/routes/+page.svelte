<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { getAllDomains, getAttentionDomains, getStatistics, getTimeseries } from '$lib/api';
	import type { DomainSummary } from '$lib/types/DomainSummary';
	import type { OnboardingState } from '$lib/types/OnboardingState';
	import type { Statistics } from '$lib/types/Statistics';
	import AttentionList from '$lib/components/attention-list.svelte';
	import BackgroundActivity from '$lib/components/background-activity.svelte';
	import VolumeChart from '$lib/components/volume-chart.svelte';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import WaitingBanner from '$lib/components/setup/waiting-banner.svelte';
	import StatCard from '$lib/components/stat-card.svelte';
	import { statisticsTrends } from '$lib/components/outcome-legend';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import MailsIcon from '@lucide/svelte/icons/mails';
	import ServerIcon from '@lucide/svelte/icons/server';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { formatRate, trendWords } from '$lib/health';
	import { formatInt, plural } from '$lib/format';
	import { resource } from '$lib/state/resource.svelte';
	import { seriesFromTimeseries, type DailyPoint } from '$lib/series';
	import { filters } from '$lib/state/filters.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';

	/**
	 * Until the setup is finished or skipped (the shared settings' onboarding flag), the app opens
	 * /setup; after it, the waiting banner shows until the first report. Health is the shared poll
	 * the header runs.
	 */
	$effect(() => {
		settings.load().catch(() => {});
	});
	/** The first settings answer (or failure) is in; until then no figures show. */
	const onboardingLoaded = $derived(settings.current !== null || settings.error !== null);
	/** `undefined` while unknown or failed: without setup state no banner shows. */
	const onboarding = $derived<OnboardingState | null | undefined>(settings.current?.onboarding.state);
	const setupOpen = $derived(onboarding === null);
	$effect(() => {
		if (setupOpen) void goto(resolve('/setup'), { replaceState: true });
	});

	const health = $derived(systemHealth.current);

	/** A large import is running (the server's `bulk`, e.g. a first sync of a full mailbox): keep its progress in view. */
	const bulkImport = $derived(health?.imap_progress?.bulk ?? false);

	/** No report covers the scope: the dashboard lists its domains instead of figures. */
	function noReportData(stats: Statistics): boolean {
		return stats.total_messages === 0 && stats.active_domains === 0;
	}

	/**
	 * The first import is running (nothing stored yet, or the server's `bulk` pass): an empty
	 * scope means its reports are still on their way, not that there are none.
	 */
	const firstImport = $derived(!!health?.imap_progress && (health.total_reports === 0 || health.imap_progress.bulk));

	/** The attention list shows this many domains; a line links to the rest. */
	const ATTENTION_ROWS = 10;

	type MetricsData = {
		stats: Statistics;
		series: DailyPoint[];
		/** The first of the scope's domains with an open issue, in their rank over the whole scope. */
		attention: DomainSummary[];
		/** How many domains have an open issue, those not in `attention` too. */
		attentionTotal: number;
		/** Every domain in the scope, fetched only while it has no report data (then listed). */
		domains: DomainSummary[];
	};

	/** The figures of one scope; a refresh or import tick keeps them on screen until the next. */
	const metrics = resource({
		key: () => `${filters.organizationId}|${filters.domainId}|${filters.since}`,
		load: async (): Promise<MetricsData> => {
			// Every panel shares one scope: organization, domain and time range.
			const scope = {
				organization_id: filters.organizationId,
				domain_id: filters.domainId,
				since: filters.since
			};
			const [stats, timeseries, attention] = await Promise.all([
				getStatistics(scope),
				getTimeseries({ ...scope, bucket: 'day' }),
				getAttentionDomains({ ...scope, limit: ATTENTION_ROWS })
			]);
			return {
				stats,
				series: seriesFromTimeseries(timeseries.items),
				attention: attention.items,
				attentionTotal: attention.total,
				domains: noReportData(stats) ? await getAllDomains(scope) : []
			};
		},
		// Reports arriving during a long mailbox import, and new domains' first DNS results
		// (both throttled to ~15 s).
		reloadOn: () => [systemHealth.importTick, systemHealth.dnsTick],
		markUpdated: true
	});

	/** The legend's skeleton keeps the trend lines' room (the figures have trends). */
	const NO_TRENDS = { compliant: null, blocked: null, forwarded: null, delivered: null };
</script>

<!-- Kept on screen while a background reload (import progress, timer) is pending. -->
{#snippet dashboard(data: MetricsData)}
	{#if noReportData(data.stats)}
		<div class="space-y-6">
			{#if firstImport}
				<EmptyState icon={InboxIcon} headline="Importing the mailbox, the first reports appear in a moment.">
					{#snippet action()}
						<BackgroundActivity imports="any" />
					{/snippet}
				</EmptyState>
			{:else}
				<EmptyState
					icon={InboxIcon}
					headline="No report data in this scope"
					description="No aggregate report covers the selected organization, domain and time range yet."
				/>
			{/if}
			{#if data.domains.length > 0}
				<section class="space-y-3">
					<h2 class="text-sm font-medium">Monitored domains</h2>
					<ul class="divide-border bg-card divide-y rounded-lg border text-sm">
						{#each data.domains as d (d.id)}
							<li>
								<a
									href={resolve(`/domains/${d.id}`)}
									class="hover:bg-muted/50 flex items-center justify-between gap-4 px-3 py-2.5 transition-colors"
								>
									<span class="font-medium">{d.domain}</span>
									<span class="text-muted-foreground text-xs">Open domain &rarr;</span>
								</a>
							</li>
						{/each}
					</ul>
				</section>
			{/if}
		</div>
	{:else}
		{@const series = data.series}
		{@const volumeTrend = trendWords(data.stats.total_messages_trend)}
		{@const complianceTrend = trendWords(data.stats.delivered_compliance_trend)}

		<div class="space-y-6">
			<div class="grid gap-3 sm:grid-cols-3">
				<StatCard
					label="Messages"
					value={formatInt(data.stats.total_messages)}
					hint="In reports"
					icon={MailsIcon}
					trend={volumeTrend}
				/>
				<StatCard
					label="Delivered compliance"
					value={formatRate(data.stats.delivered_compliance)}
					hint="Of delivered mail"
					icon={ShieldCheckIcon}
					trend={complianceTrend}
				/>
				<StatCard
					label="Senders"
					value={formatInt(data.stats.unique_senders)}
					hint={`Source IPs, ${plural(data.stats.active_domains, 'domain')}`}
					icon={ServerIcon}
				/>
			</div>

			<!-- No total in the legend: the Messages card above has it, with its trend. -->
			<VolumeChart {series} outcomes={{ mail: data.stats, trends: statisticsTrends(data.stats) }} />

			<AttentionList domains={data.attention} total={data.attentionTotal} />
		</div>
	{/if}
{/snippet}

{#snippet loadingDashboard()}
	<div class="space-y-6">
		<div class="grid gap-3 sm:grid-cols-3">
			<StatCard label="Messages" loading />
			<StatCard label="Delivered compliance" loading />
			<StatCard label="Senders" loading />
		</div>
		<VolumeChart series={[]} loading outcomes={{ mail: null, trends: NO_TRENDS }} />
	</div>
{/snippet}

{#if !onboardingLoaded || setupOpen}
	<!-- A fresh install opens the setup: do not flash figures before the setup state is known. -->
	{@render loadingDashboard()}
{:else}
	{@const banner = onboarding !== undefined && health && (health.total_reports === 0 || bulkImport)}
	{#if banner && health}
		<div class="mb-6">
			<WaitingBanner {health} mailboxConfigured={health.imap_configured} />
		</div>
	{/if}
	<!-- The banner shows a running import itself; the DNS checks of new domains show here. -->
	<BackgroundActivity imports={banner ? 'none' : 'bulk'} class="mb-4" />
	<Loaded source={metrics} errorTitle="Dashboard failed to load" data={dashboard} loading={loadingDashboard} />
{/if}
