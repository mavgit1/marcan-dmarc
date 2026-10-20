<script lang="ts">
	import { cn } from '$lib/utils.js';
	import type { Snippet } from 'svelte';

	/**
	 * Page title block used at the top of a route.
	 * Put primary actions (buttons) in the `actions` snippet. The breadcrumb trail lives in the
	 * app header (`crumbsForPath`), never here.
	 */
	let {
		title,
		description,
		titleAddon,
		below,
		actions,
		class: className
	}: {
		title: string;
		description?: string;
		/** Small tag rendered inline after the title (e.g. "Default"). */
		titleAddon?: Snippet;
		/** A line under the description that belongs to the title (e.g. a domain's policy readiness). */
		below?: Snippet;
		actions?: Snippet;
		class?: string;
	} = $props();
</script>

<header class={cn('mb-6 flex flex-wrap items-start justify-between gap-3', className)}>
	<div class="min-w-0 space-y-1">
		{#if titleAddon}
			<div class="flex flex-wrap items-center gap-2">
				<h1 class="text-xl font-semibold tracking-tight">{title}</h1>
				{@render titleAddon()}
			</div>
		{:else}
			<h1 class="text-xl font-semibold tracking-tight">{title}</h1>
		{/if}
		{#if description}
			<p class="text-muted-foreground max-w-2xl text-sm">{description}</p>
		{/if}
		{@render below?.()}
	</div>
	{#if actions}
		<!-- At most the header's width, so the actions wrap on a phone instead of widening the page. -->
		<div class="flex max-w-full shrink-0 flex-wrap items-center gap-2">
			{@render actions()}
		</div>
	{/if}
</header>
