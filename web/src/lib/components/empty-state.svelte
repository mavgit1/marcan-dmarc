<script lang="ts">
	import { cn } from '$lib/utils.js';
	import type { Component, Snippet } from 'svelte';

	/**
	 * Calm empty placeholder for tables, pages, and panels.
	 * Pass an icon component (Lucide) and optional `action` snippet for a CTA.
	 */
	let {
		icon: Icon,
		headline,
		description,
		action,
		class: className
	}: {
		icon?: Component<{ class?: string }>;
		headline: string;
		description?: string;
		action?: Snippet;
		class?: string;
	} = $props();
</script>

<div class={cn('flex flex-col items-center justify-center gap-2 px-6 py-12 text-center', className)}>
	{#if Icon}
		<div class="bg-muted text-muted-foreground mb-1 flex size-10 items-center justify-center rounded-lg">
			<Icon class="size-5" />
		</div>
	{/if}
	<p class="text-sm font-medium">{headline}</p>
	{#if description}
		<p class="text-muted-foreground max-w-sm text-sm">{description}</p>
	{/if}
	{#if action}
		<div class="mt-2">
			{@render action()}
		</div>
	{/if}
</div>
