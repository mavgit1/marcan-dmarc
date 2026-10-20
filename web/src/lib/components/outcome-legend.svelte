<script lang="ts">
	import Skeleton from '$lib/components/skeleton.svelte';
	import TrendLine from '$lib/components/trend-line.svelte';
	import { cn } from '$lib/utils.js';
	import { OUTCOMES } from '$lib/wording/outcome';
	import { legendCells, legendTotal, type OutcomeFigures } from './outcome-legend';

	/**
	 * The one key to the four outcomes, with their counts: a chart's footer (`VolumeChart`
	 * renders it when given `outcomes`) and, without a chart, a sender's domain row and a
	 * report. Four cells of equal width in `OUTCOMES` order (swatch and label, then the count
	 * with the trend after it when given), the total on the left when `total` is given, set apart from the cells
	 * by a hairline and its stat-card label so it never reads as a fifth cell. The layout is the
	 * container's, not the viewport's: narrow, the total sits above a 2 by 2 grid. Pages pass
	 * data, never layout.
	 */
	let { mail, total: totalWords, trends, class: className }: OutcomeFigures & { class?: string } = $props();

	const cells = $derived(mail ? legendCells(mail, trends) : null);
	const withTotal = $derived(totalWords != null);
	const total = $derived(
		mail && totalWords != null ? legendTotal(mail, totalWords === true ? undefined : totalWords) : null
	);
</script>

<div class={cn('@container', className)}>
	<div class={cn('flex flex-col gap-3', withTotal && '@4xl:flex-row @4xl:items-center @4xl:gap-5')}>
		{#if withTotal}
			<div class="border-b pb-3 @4xl:shrink-0 @4xl:border-r @4xl:border-b-0 @4xl:py-1 @4xl:pr-5">
				{#if total}
					<span class="text-muted-foreground block text-xs leading-tight font-medium tracking-wide uppercase">
						{total.label}
					</span>
					<span class="mt-0.5 block text-base font-semibold tabular-nums">{total.count}</span>
				{:else}
					<div class="space-y-1.5">
						<Skeleton class="h-3 w-28" />
						<Skeleton class="h-5 w-16" />
					</div>
				{/if}
			</div>
		{/if}
		<ul class="grid min-w-0 flex-1 grid-cols-2 gap-x-4 gap-y-3 @xl:grid-cols-4">
			{#if cells}
				{#each cells as c (c.key)}
					<li class="min-w-0" title={c.title}>
						<span class="text-muted-foreground flex items-center gap-1.5 text-xs leading-tight">
							<span class={cn('size-2.5 shrink-0 rounded-[2px]', c.swatch, c.muted && 'opacity-40')} aria-hidden="true"
							></span>
							{c.label}
						</span>
						<span class="mt-0.5 flex flex-wrap items-baseline gap-x-2.5">
							<span class={cn('text-base font-semibold tabular-nums', c.muted && 'text-muted-foreground font-medium')}>
								{c.count}
							</span>
							{#if c.trend}
								<TrendLine trend={c.trend} />
							{/if}
						</span>
					</li>
				{/each}
			{:else}
				{#each OUTCOMES as o (o.key)}
					<li class="space-y-1.5" aria-hidden="true">
						<Skeleton class="h-3 w-24" />
						<Skeleton class={cn('h-5', trends ? 'w-32' : 'w-14')} />
					</li>
				{/each}
			{/if}
		</ul>
	</div>
</div>
