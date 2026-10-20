<script lang="ts">
	import MinusIcon from '@lucide/svelte/icons/minus';
	import TrendingDownIcon from '@lucide/svelte/icons/trending-down';
	import TrendingUpIcon from '@lucide/svelte/icons/trending-up';
	import type { StatTrend } from '$lib/health';
	import { statusText } from '$lib/tokens';
	import { cn } from '$lib/utils.js';

	/**
	 * A figure's trend against the previous window (`trendWords`), as `StatCard` and
	 * `OutcomeLegend` show it on the count's line, after the count with a gap: "+81.7% ↗", the
	 * signed figure, then the arrow (the direction), no parentheses and no pill. The colour is
	 * the server's `tone`, what it is compared with the tooltip.
	 */
	let { trend, class: className }: { trend: StatTrend; class?: string } = $props();
</script>

<span
	class={cn(
		'inline-flex items-center text-xs font-medium whitespace-nowrap tabular-nums',
		statusText[trend.tone],
		className
	)}
	title={trend.title}
>
	{trend.label}
	{#if trend.direction === 'up'}
		<TrendingUpIcon class="ml-1 size-3 shrink-0" />
	{:else if trend.direction === 'down'}
		<TrendingDownIcon class="ml-1 size-3 shrink-0" />
	{:else}
		<MinusIcon class="ml-1 size-3 shrink-0" />
	{/if}
</span>
