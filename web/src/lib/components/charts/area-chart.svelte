<script lang="ts">
	import { scaleUtc } from 'd3-scale';
	import { AreaChart as LayerAreaChart } from 'layerchart';
	import * as Chart from '$lib/components/ui/chart/index.js';
	import { formatChartDate, toChartConfig, type ChartDatum, type ChartSeries } from './chart-model.js';
	import { cn } from '$lib/utils.js';

	/**
	 * Theme-aware area chart. Pages should not import layerchart directly.
	 */
	let {
		data,
		x,
		series,
		xScale,
		xFormat,
		yFormat,
		legend = false,
		seriesLayout = 'stack',
		tickSpacing = 80,
		padding,
		class: className
	}: {
		data: ChartDatum[];
		x: string;
		series: ChartSeries[];
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		xScale?: any;
		xFormat?: (d: unknown) => string;
		yFormat?: (d: unknown) => string;
		legend?: boolean;
		seriesLayout?: 'auto' | 'stack' | 'overlap' | 'stackExpand' | 'stackDiverging';
		tickSpacing?: number;
		padding?: { top?: number; right?: number; bottom?: number; left?: number };
		class?: string;
	} = $props();

	const config = $derived(toChartConfig(series));
	const mapped = $derived(
		series.map((s) => ({
			key: s.key,
			label: s.label,
			color: `var(--color-${s.key})`,
			value: s.key
		}))
	);

	const isDateAxis = $derived(data.length > 0 && data[0][x] instanceof Date);
	const resolvedXScale = $derived.by(() => {
		if (xScale) return xScale;
		if (isDateAxis) return scaleUtc();
		return undefined;
	});
	const resolvedXFormat = $derived(xFormat ?? (isDateAxis ? formatChartDate : undefined));

	const resolvedPadding = $derived.by(() => {
		const base = {
			top: 10,
			right: 28,
			bottom: 24,
			left: 36
		};
		return { ...base, ...padding };
	});
</script>

<!-- HTML legend instead of layerchart's: it wraps on narrow screens. -->
<Chart.Container {config} class={cn('aspect-auto h-[240px] w-full', className)}>
	<LayerAreaChart
		{data}
		{x}
		axis
		series={mapped}
		{seriesLayout}
		xScale={resolvedXScale}
		padding={resolvedPadding}
		props={{
			xAxis: {
				...(resolvedXFormat ? { format: resolvedXFormat } : {}),
				tickSpacing
			},
			yAxis: yFormat ? { format: yFormat } : {}
		}}
	>
		{#snippet tooltip()}
			<Chart.Tooltip labelFormatter={resolvedXFormat} />
		{/snippet}
	</LayerAreaChart>
</Chart.Container>
{#if legend}
	<ul class="text-muted-foreground flex flex-wrap justify-center gap-x-4 gap-y-1 text-xs">
		{#each series as s, i (s.key)}
			<li class="flex items-center gap-1.5">
				<span class="size-2.5 rounded-[2px]" style:background={s.color ?? `var(--chart-${(i % 5) + 1})`}></span>
				{s.label}
			</li>
		{/each}
	</ul>
{/if}
