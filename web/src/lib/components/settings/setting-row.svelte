<script lang="ts">
	import { cn } from '#lib/utils.ts';
	import type { Snippet } from 'svelte';
	import { onFocusAnchor } from '#lib/state/anchor-focus.svelte.ts';

	/**
	 * One setting: its name beside the control, and a hint (only where the name is not enough)
	 * under the control, the full width of its cell, on every row alike: the name column holds the
	 * name alone, so no hint wraps in it. `stacked` puts wide controls (forms, editors) below the
	 * name instead. Below `md` the name always sits above the control. The control cell is a size
	 * container: grids inside use container variants (`@lg:grid-cols-…`), so they stack whenever
	 * the cell is narrow, whatever the viewport (a sidebar beside a two-column row leaves little
	 * room). A switch row passes `labelFor`, the checkbox's id: the title is its label. `anchor`
	 * is the id a link opens (a section of `SETTINGS_SECTIONS` that is one row).
	 */
	let {
		title,
		hint,
		labelFor,
		stacked = false,
		anchor,
		children
	}: {
		title: string;
		hint?: string | Snippet;
		/** The id of the one control the title labels (a switch row's checkbox). */
		labelFor?: string;
		stacked?: boolean;
		anchor?: string;
		children: Snippet;
	} = $props();

	let element = $state<HTMLElement | null>(null);
	onFocusAnchor(
		() => anchor,
		() => element
	);
</script>

<div
	bind:this={element}
	id={anchor}
	class={cn(
		'grid scroll-mt-below-header gap-3 px-5 py-4',
		!stacked && 'md:grid-cols-[minmax(0,14rem)_minmax(0,1fr)] md:items-start md:gap-8'
	)}
>
	<h3 class="min-w-0 text-sm font-semibold break-words">
		{#if labelFor}<label for={labelFor} class="cursor-pointer">{title}</label>{:else}{title}{/if}
	</h3>
	<!-- A container: the controls lay out by the room this cell has, not by the viewport. -->
	<div class="@container min-w-0 text-sm break-words">
		{@render children()}
		{#if typeof hint === 'string'}
			<p class="text-muted-foreground mt-1.5 text-xs leading-relaxed">{hint}</p>
		{:else if hint}
			<p class="text-muted-foreground mt-1.5 text-xs leading-relaxed">{@render hint()}</p>
		{/if}
	</div>
</div>
