<script lang="ts" module>
	import { CHECK_PILLAR, PILLAR_IDS, PILLAR_LABEL, type PillarId } from '$lib/dns';

	/**
	 * The pillar each attention-list kind opens (`?focus=<kind>`); `worst` opens the pillar of
	 * the worst DNS finding once the check is in. The one kind-to-pillar map.
	 */
	export const ATTENTION_FOCUS = {
		dns: 'worst',
		delivered_compliance: 'telemetry',
		enforcement: 'dmarc'
	} as const satisfies Record<string, PillarId | 'worst'>;
	export type AttentionKind = keyof typeof ATTENTION_FOCUS;

	/** `?focus=` values mapped to the pillar they open. */
	const FOCUS_ALIASES: Record<string, PillarId | 'worst'> = {
		...ATTENTION_FOCUS,
		dmarc: 'dmarc',
		spf: 'spf',
		dkim: 'dkim',
		transport: 'transport',
		'mta-sts': 'transport',
		mtasts: 'transport',
		'tls-rpt': 'transport',
		tlsrpt: 'transport',
		bimi: 'transport',
		telemetry: 'telemetry',
		failures: 'telemetry'
	};
</script>

<script lang="ts">
	import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import PillarCard from '$lib/components/dns/pillar-card.svelte';
	import ScorecardDrawer from '$lib/components/dns/scorecard-drawer.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { DELIVERED_FAILURE_FILTER, getDomainFailures } from '$lib/api';
	import { pillarModels } from '$lib/dns';
	import { filters } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { formatCheckedAge } from '$lib/format';
	import { capitalize } from '$lib/text';
	import { statusSurface } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import Loader2Icon from '@lucide/svelte/icons/loader-2';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';

	let {
		overview,
		dns = null,
		dnsLoading = false,
		dnsError = null,
		initialFocus = null,
		onRefresh
	}: {
		overview: DomainOverview;
		dns?: DnsCheckResponse | null;
		dnsLoading?: boolean;
		dnsError?: string | null;
		initialFocus?: string | null;
		onRefresh?: () => void;
	} = $props();

	let activePillar = $state<PillarId | null>(null);

	// Top delivered failures for the Report data drawer: DMARC failed and delivered, the
	// records behind `delivered_failures`, in the page's window. Loaded when that drawer
	// opens and kept here, so reopening it shows the last rows while they reload. Keyed on
	// the id, not the object: each refresh brings a new `overview`.
	const overviewId = $derived(overview.id);
	const topFailures = resource({
		key: () => `${overviewId}|${filters.since}`,
		load: () =>
			activePillar === 'telemetry'
				? getDomainFailures(overviewId, { limit: 5, since: filters.since, ...DELIVERED_FAILURE_FILTER })
				: null,
		reloadOn: () => activePillar
	});
	const failures = $derived(topFailures.data?.items ?? (topFailures.error ? [] : null));

	/** Each `?focus=` is applied once, so a later DNS refresh does not reopen a closed drawer. */
	let appliedFocus: string | null = null;
	$effect(() => {
		const key = initialFocus?.toLowerCase().trim();
		if (!key || key === appliedFocus || !Object.hasOwn(FOCUS_ALIASES, key)) return;
		const target = FOCUS_ALIASES[key];
		if (target === 'worst') {
			if (!dns) return; // wait for the check
			activePillar = CHECK_PILLAR[dns.worst_check];
		} else {
			activePillar = target;
		}
		appliedFocus = key;
	});

	function togglePillar(pillar: PillarId) {
		activePillar = activePillar === pillar ? null : pillar;
	}

	/** Every card and drawer, from the check and the overview (`$lib/dns`). */
	const pillars = $derived(pillarModels(dns, overview));

	/** No answer yet: the DNS cards show skeletons. A failed check reads "Check failed". */
	const dnsPending = $derived(!dns && !dnsError);
	/** The shown result is the last complete one; the latest attempt failed. */
	const dnsStale = $derived(dns?.status === 'stale');
	const dnsUpdating = $derived(dns?.status === 'updating');

	// Overall grade: no letter when the request failed or the server says no complete
	// measurement backs it (`graded`).
	const gradeUnavailable = $derived(!dnsLoading && (dnsError != null || (dns != null && !dns.graded)));
	const gradeSurface = $derived(
		gradeUnavailable ? statusSurface.neutral : statusSurface[dns?.grade_status ?? 'neutral']
	);
</script>

<div class="space-y-3">
	<div
		class="flex flex-col gap-3 rounded-lg border border-border bg-card p-4 sm:flex-row sm:items-center sm:justify-between"
	>
		<div class="flex items-center gap-3">
			<div
				class={cn(
					'flex size-11 items-center justify-center rounded-md border text-xl font-bold tracking-tight',
					gradeSurface
				)}
			>
				{gradeUnavailable ? '?' : (dns?.grade ?? '—')}
			</div>
			<div>
				<div class="flex flex-wrap items-center gap-2">
					<h2 class="text-sm font-medium text-foreground">DNS checks</h2>
					{#if dns && !gradeUnavailable}
						<span class="text-xs text-muted-foreground tabular-nums">Score {dns.score} of 100</span>
					{/if}
					{#if dnsLoading}
						<Badge
							variant="outline"
							class="flex items-center gap-1.5 border-border text-muted-foreground text-xs font-normal"
						>
							<Loader2Icon class="size-3 animate-spin" />
							Checking live DNS…
						</Badge>
					{:else if dns?.cached}
						<span class="text-[11px] text-muted-foreground">(cached)</span>
					{/if}
				</div>
				<p class="text-muted-foreground mt-0.5 text-xs">
					{#if !dnsLoading && dnsError}
						<span class="text-status-fail-fg" role="alert">DNS check failed: {dnsError}</span>
					{:else if gradeUnavailable}
						Check failed · retrying
					{:else if dnsUpdating && dns?.checked_at}
						<span title="Marcan's DNS checks changed since this result; it is being re-checked">
							{capitalize(formatCheckedAge(dns.checked_at))} · updating to the current checks
						</span>
					{:else if dnsStale && dns?.checked_at}
						<span class="text-status-warn-fg">
							{capitalize(formatCheckedAge(dns.checked_at))} · update failed, retrying
						</span>
					{:else if dns?.checked_at}
						Score reflects DNS configuration only, {formatCheckedAge(dns.checked_at)}.
					{:else}
						Checking DNS configuration.
					{/if}
				</p>
			</div>
		</div>

		<div class="flex items-center gap-2 self-start sm:self-center">
			{#if onRefresh}
				<Button
					variant="outline"
					size="sm"
					class="gap-1.5 text-xs h-8 px-2.5"
					disabled={dnsLoading}
					onclick={onRefresh}
				>
					<RefreshCwIcon class={cn('size-3.5', dnsLoading && 'animate-spin')} />
					Re-check DNS
				</Button>
			{/if}
		</div>
	</div>

	<div class="grid gap-3 grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-5">
		{#each PILLAR_IDS as id (id)}
			{@const label = PILLAR_LABEL[id]}
			{@const model = pillars[id]}
			<PillarCard
				{label}
				lostPts={model.lostPts}
				verdict={model.verdict}
				verdictLabel={model.verdictLabel}
				findings={model.findingsVerdict}
				pending={id !== 'telemetry' && dnsPending}
				active={activePillar === id}
				onclick={() => togglePillar(id)}
				rows={model.rows}
			/>
		{/each}
	</div>

	{#if activePillar}
		<ScorecardDrawer
			pillar={activePillar}
			model={pillars[activePillar]}
			{dns}
			{overview}
			{failures}
			{dnsLoading}
			{onRefresh}
			onClose={() => (activePillar = null)}
		/>
	{/if}
</div>
