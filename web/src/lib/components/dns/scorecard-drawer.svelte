<script lang="ts">
	import { resolve } from '$app/paths';
	import { DELIVERED_FAILURE_FILTER, outcomeParam } from '$lib/api';
	import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
	import type { DomainFailureRow } from '$lib/types/DomainFailureRow';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import type { WithIdentity } from '$lib/types/WithIdentity';
	import DkimDetails from '$lib/components/dns/dkim-details.svelte';
	import DmarcDetails from '$lib/components/dns/dmarc-details.svelte';
	import FindingsSection from '$lib/components/dns/findings-section.svelte';
	import SpfDetails from '$lib/components/dns/spf-details.svelte';
	import TelemetryDetails from '$lib/components/dns/telemetry-details.svelte';
	import TransportDetails from '$lib/components/dns/transport-details.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { PILLAR_LABEL, type PillarId, type PillarModel } from '$lib/dns';
	import XIcon from '@lucide/svelte/icons/x';

	/**
	 * The scorecard's inspection drawer for one pillar: what is wrong first (worst first), the
	 * record and technical detail below. The telemetry pillar shows the top delivered failures
	 * the scorecard loads (it owns them, so reopening the drawer keeps the last rows).
	 */
	let {
		pillar,
		model,
		dns,
		overview,
		failures,
		dnsLoading = false,
		onRefresh,
		onClose
	}: {
		pillar: PillarId;
		model: PillarModel;
		dns: DnsCheckResponse | null;
		overview: DomainOverview;
		/** `null` while loading; `[]` when the request failed. */
		failures: WithIdentity<DomainFailureRow>[] | null;
		dnsLoading?: boolean;
		onRefresh?: () => void;
		onClose: () => void;
	} = $props();

	const SUBTITLE: Record<PillarId, string> = {
		dmarc: 'Policy and report destinations (RFC 7489).',
		spf: 'Include tree and the 10-lookup limit (RFC 7208).',
		dkim: 'Keys of the selectors seen signing (RFC 6376).',
		transport: 'MTA-STS (RFC 8461), TLS-RPT (RFC 8460) and BIMI.',
		telemetry: 'Delivered failures and their sources. Not part of the score.'
	};
</script>

<div class="rounded-lg border border-border bg-card p-4 shadow-xs">
	<div class="mb-4 flex items-center justify-between border-b border-border pb-3">
		<div>
			<h3 class="text-sm font-semibold text-foreground">{PILLAR_LABEL[pillar]}</h3>
			<p class="text-muted-foreground text-xs mt-0.5">{SUBTITLE[pillar]}</p>
		</div>
		{#if pillar === 'dmarc'}
			<a
				href="{resolve('/guide')}#dmarc"
				class="text-muted-foreground hover:text-foreground ml-auto mr-2 text-xs underline underline-offset-2"
			>
				Setup guide
			</a>
		{/if}
		<Button
			variant="ghost"
			size="sm"
			class="size-7 p-0 text-muted-foreground hover:text-foreground"
			onclick={onClose}
			aria-label="Close inspection drawer"
		>
			<XIcon class="size-4" />
		</Button>
	</div>

	<!-- What is wrong first, worst first; the record and technical detail below. -->
	<div class="mb-5">
		<FindingsSection title="Findings" findings={model.findings} showDeductions={pillar !== 'telemetry'} />
	</div>
	{#if pillar === 'dmarc'}
		<DmarcDetails check={dns?.dmarc} domain={overview.domain} loading={dnsLoading} {onRefresh} />
	{:else if pillar === 'spf'}
		<SpfDetails check={dns?.spf} domain={overview.domain} />
	{:else if pillar === 'dkim'}
		<DkimDetails check={dns?.dkim} {onRefresh} />
	{:else if pillar === 'transport'}
		<TransportDetails {dns} domain={overview.domain} />
	{:else}
		<TelemetryDetails
			failuresHref={`${resolve(`/domains/${overview.id}`)}?tab=failures&outcome=${outcomeParam(DELIVERED_FAILURE_FILTER.outcome)}`}
			{failures}
		/>
	{/if}
</div>
