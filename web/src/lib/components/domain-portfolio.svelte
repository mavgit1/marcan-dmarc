<script lang="ts">
	import { getStatistics } from '$lib/api';
	import type { Statistics } from '$lib/types/Statistics';
	import DomainsTable from '$lib/components/domains-table.svelte';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import StatCard from '$lib/components/stat-card.svelte';
	import { formatInt } from '$lib/format';
	import { formatRate } from '$lib/health';
	import { outcomeCountLine } from '$lib/wording/outcome';
	import { filters } from '$lib/state/filters.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import MailsIcon from '@lucide/svelte/icons/mails';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import type { Snippet } from 'svelte';

	/**
	 * Stat cards plus the shared domain table for one scope: every organization
	 * (`organizationId` null) or a single one. Both follow the global time range, and both
	 * come from the same scope, so the rows add up to the cards. The cards count the whole
	 * scope; the table pages and filters on its own, and while its filter narrows it a line under
	 * the cards says they cover all domains (the statistics take no filter).
	 */
	let {
		organizationId,
		showOrganization = true,
		emptyHeadline = 'No domains yet',
		emptyDescription,
		afterFigures,
		attention = false,
		onClearAttention
	}: {
		organizationId: number | null;
		showOrganization?: boolean;
		emptyHeadline?: string;
		emptyDescription?: string;
		/** Between the cards and the table (the organization page's client report line). */
		afterFigures?: Snippet;
		/** The table shows only the domains with an open issue (the Domains page's `?attention=1`). */
		attention?: boolean;
		/** Clears that filter. */
		onClearAttention?: () => void;
	} = $props();

	/**
	 * A refresh keeps the figures on screen (adding a domain should not flash skeletons);
	 * a scope change shows skeletons.
	 */
	const stats = resource({
		key: () => `${organizationId}|${filters.since}`,
		load: (): Promise<Statistics> => getStatistics({ organization_id: organizationId, since: filters.since }),
		// New domains during an import (throttled to ~15 s).
		reloadOn: () => [systemHealth.importTick, systemHealth.dnsTick],
		markUpdated: true
	});

	/** The table's filter box narrows the list; the cards stay on the whole scope. */
	let filtered = $state(false);
</script>

{#snippet figures(data: Statistics)}
	<div class={['grid grid-cols-1 gap-4 sm:grid-cols-3', filtered ? 'mb-2' : 'mb-6']}>
		<StatCard label="Messages" value={formatInt(data.total_messages)} hint={outcomeCountLine(data)} icon={MailsIcon} />
		<StatCard
			label="Delivered compliance"
			value={formatRate(data.delivered_compliance)}
			hint="Of delivered mail"
			icon={ShieldCheckIcon}
		/>
		<StatCard
			label="Domains"
			value={formatInt(data.domain_count)}
			hint={`${formatInt(data.active_domains)} with reports in range`}
			icon={GlobeIcon}
		/>
	</div>
	{#if filtered}
		<p class="text-muted-foreground mb-6 text-xs">
			These figures cover all domains; the filter narrows the list below.
		</p>
	{/if}
{/snippet}

{#snippet loading()}
	<div class="mb-6 grid grid-cols-1 gap-4 sm:grid-cols-3">
		<StatCard label="Messages" loading />
		<StatCard label="Delivered compliance" loading />
		<StatCard label="Domains" loading />
	</div>
{/snippet}

<Loaded source={stats} errorTitle="Failed to load the figures" data={figures} {loading} />

{@render afterFigures?.()}

<!-- Mounted once, outside the cards' load, so it fetches its first page only once. -->
{#if stats.data?.domain_count === 0}
	<EmptyState icon={GlobeIcon} headline={emptyHeadline} description={emptyDescription} />
{:else}
	<DomainsTable {organizationId} {showOrganization} {attention} {onClearAttention} bind:filtered />
{/if}
