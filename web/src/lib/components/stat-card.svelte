<script lang="ts">
	import * as Card from '$lib/components/ui/card/index.js';
	import Skeleton from '$lib/components/skeleton.svelte';
	import TrendLine from '$lib/components/trend-line.svelte';
	import type { StatTrend } from '$lib/health';
	import { cn } from '$lib/utils.js';
	import type { Component, Snippet } from 'svelte';

	/**
	 * Compact metric tile: the label, the value with its trend after it on the same line
	 * ("23,459   +48.8% ↗", a clear gap between them), the hint under it. The trend's colour is its `tone` (not its
	 * direction), so “up” can be bad (failures) or good (compliance); a flat trend is muted.
	 */
	let {
		label,
		value,
		hint,
		icon: Icon,
		trend,
		sparkline,
		loading = false,
		class: className
	}: {
		label: string;
		value?: string | number;
		hint?: string;
		icon?: Component<{ class?: string }>;
		trend?: StatTrend;
		sparkline?: Snippet;
		loading?: boolean;
		class?: string;
	} = $props();
</script>

<Card.Root size="sm" class={cn('gap-3', className)}>
	<Card.Header class="flex flex-row items-center justify-between gap-2">
		<Card.Title class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
			{label}
		</Card.Title>
		{#if Icon}
			<Icon class="text-muted-foreground size-3.5" />
		{/if}
	</Card.Header>
	<Card.Content class="space-y-1">
		{#if loading}
			<Skeleton class="h-7 w-24" />
			<Skeleton class="h-3 w-16" />
		{:else}
			<p class="flex flex-wrap items-baseline gap-x-3">
				<span class="text-2xl font-semibold tracking-tight tabular-nums">{value ?? '—'}</span>
				{#if trend}
					<TrendLine {trend} />
				{/if}
			</p>
			{#if hint || sparkline}
				<div class="flex items-center justify-between gap-2">
					{#if hint}
						<p class="text-muted-foreground text-xs">{hint}</p>
					{/if}
					{#if sparkline}
						<div class="h-8 w-20 shrink-0">
							{@render sparkline()}
						</div>
					{/if}
				</div>
			{/if}
		{/if}
	</Card.Content>
</Card.Root>
