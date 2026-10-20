<script lang="ts">
	import AreaChart from '$lib/components/charts/area-chart.svelte';
	import { formatChartDate } from '$lib/components/charts/chart-model';
	import EmptyState from '$lib/components/empty-state.svelte';
	import OutcomeLegend from '$lib/components/outcome-legend.svelte';
	import type { OutcomeFigures } from '$lib/components/outcome-legend';
	import { OUTCOMES } from '$lib/wording/outcome';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { formatInt } from '$lib/format';
	import type { DailyPoint } from '$lib/series';
	import { cn } from '$lib/utils.js';
	import InboxIcon from '@lucide/svelte/icons/inbox';

	/**
	 * Daily message volume split into the four outcomes, in the colours every tile and badge
	 * uses: DMARC pass green, blocked blue, forwarded violet, delivered failures red. Given
	 * `outcomes`, its key is the `OutcomeLegend` under a rule (the counts, and the total or
	 * trends the page asks for) in place of the chart's own legend.
	 */
	let {
		series,
		title = 'Messages per day',
		loading = false,
		outcomes,
		class: className
	}: {
		series: DailyPoint[];
		title?: string;
		loading?: boolean;
		/** The range's counts by outcome: the legend with numbers, in the chart's footer. */
		outcomes?: OutcomeFigures;
		class?: string;
	} = $props();
</script>

<section class={cn('bg-card ring-foreground/10 space-y-3 rounded-xl p-4 ring-1', className)}>
	<div class="flex flex-wrap items-baseline justify-between gap-2">
		<h2 class="text-sm font-medium">{title}</h2>
		<p class="text-muted-foreground text-xs">Daily buckets, UTC</p>
	</div>
	{#if loading}
		<Skeleton variant="block" class="h-[240px]" />
	{:else if series.length === 0}
		<EmptyState
			icon={InboxIcon}
			headline="No trend yet"
			description="Reports in this scope do not cover enough days to chart."
			class="py-8"
		/>
	{:else}
		<AreaChart
			data={series}
			x="dateObj"
			legend={!outcomes}
			xFormat={formatChartDate}
			yFormat={(d) => formatInt(Number(d ?? 0))}
			series={OUTCOMES.map((o) => ({ key: o.key, label: o.label, color: o.cssVar }))}
		/>
	{/if}
	{#if outcomes}
		<OutcomeLegend class="border-t pt-3" {...outcomes} />
	{/if}
</section>
