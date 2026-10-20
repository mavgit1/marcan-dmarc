<script lang="ts">
	import type { ImapProgressResponse } from '$lib/types/ImapProgressResponse';
	import { formatInt } from '$lib/format';
	import { progressLabel, progressPercent } from '$lib/imap-progress';
	import { cn } from '$lib/utils.js';

	/** A slim bar with "processed / total" beside it; before importing, the phase in place of the count. */
	let { progress, class: className }: { progress: ImapProgressResponse; class?: string } = $props();

	const pct = $derived(progressPercent(progress));
	const importing = $derived(progress.phase === 'importing' && pct != null);
	const label = $derived(progressLabel(progress));
</script>

<div class={cn('flex items-center gap-2', className)} role="status" aria-live="polite">
	<div
		class="bg-muted h-1.5 min-w-0 flex-1 overflow-hidden rounded-full"
		role="progressbar"
		aria-label={label}
		aria-valuemin={0}
		aria-valuemax={100}
		aria-valuenow={importing && pct != null ? Math.floor(pct) : undefined}
	>
		{#if importing && pct != null}
			<div class="bg-primary h-full rounded-full transition-[width] duration-500" style:width="{pct}%"></div>
		{:else}
			<div class="bg-primary/60 h-full w-1/3 animate-pulse rounded-full"></div>
		{/if}
	</div>
	<span class="text-muted-foreground shrink-0 text-[11px] tabular-nums">
		{#if importing}
			{formatInt(progress.processed_messages)} / {formatInt(progress.total_messages)}
		{:else}
			{label}
		{/if}
	</span>
</div>
