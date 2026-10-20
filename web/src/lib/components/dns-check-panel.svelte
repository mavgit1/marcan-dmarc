<script lang="ts">
	import ErrorState from '$lib/components/error-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { cn } from '$lib/utils.js';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import type { Snippet } from 'svelte';

	type DnsPanelStatus = 'idle' | 'loading' | 'ready' | 'error';

	/**
	 * Independent loading shell for one DNS check. Sibling panels keep rendering
	 * when this one is still loading or has failed.
	 */
	let {
		title,
		status,
		errorMessage = null,
		refreshing = false,
		onRetry,
		header,
		children,
		collapsible = true,
		open = $bindable(true),
		class: className
	}: {
		title: string;
		status: DnsPanelStatus;
		errorMessage?: string | null;
		/** A re-check is running while the last result stays on screen. */
		refreshing?: boolean;
		onRetry?: () => void;
		header?: Snippet;
		children?: Snippet;
		collapsible?: boolean;
		open?: boolean;
		class?: string;
	} = $props();

	function toggle() {
		if (collapsible) {
			open = !open;
		}
	}
</script>

<section
	class={cn('rounded-lg border border-border bg-card transition-all', className)}
	aria-busy={status === 'loading' || refreshing}
>
	<header class={cn('flex flex-wrap items-center justify-between gap-2 px-4 py-3', open && 'border-b border-border')}>
		{#if collapsible}
			<button
				type="button"
				class="flex items-center gap-2 text-left select-none hover:opacity-80 transition-opacity cursor-pointer"
				onclick={toggle}
				aria-expanded={open}
			>
				<ChevronDownIcon
					class={cn('text-muted-foreground size-4 shrink-0 transition-transform duration-200', !open && '-rotate-90')}
				/>
				<h2 class="text-sm font-medium">{title}</h2>
			</button>
		{:else}
			<div class="flex items-center gap-2">
				<h2 class="text-sm font-medium">{title}</h2>
			</div>
		{/if}
		{#if refreshing || header}
			<div class="text-muted-foreground flex flex-wrap items-center gap-2 text-xs">
				{#if refreshing}
					<span>Checking…</span>
				{/if}
				{#if header}
					{@render header()}
				{/if}
			</div>
		{/if}
	</header>

	{#if open}
		<div class="space-y-4 px-4 py-4">
			{#if status === 'idle'}
				<p class="text-muted-foreground text-sm">Enter a domain to run this check.</p>
			{:else if status === 'loading'}
				<div class="space-y-2" aria-label="{title} loading">
					<Skeleton variant="text" class="w-1/3" />
					<Skeleton variant="block" class="h-20" />
					<Skeleton variant="text" class="w-2/3" />
				</div>
			{:else if status === 'error'}
				<ErrorState title="{title} check failed" message={errorMessage ?? 'The request failed.'} {onRetry} />
			{:else if children}
				{@render children()}
			{/if}
		</div>
	{/if}
</section>
