<script lang="ts">
	import CopyButton from '$lib/components/copy-button.svelte';
	import { cn } from '$lib/utils.js';

	/**
	 * Selectable, wrapping monospace DNS record text with an optional copy control.
	 * `emphasis`: `primary` for the one record a page recommends (a card, larger text and a
	 * "Copy record" button), `secondary` for the records beside it (smaller, muted).
	 */
	let {
		value,
		label = 'Record',
		emphasis,
		class: className
	}: {
		value: string;
		label?: string;
		emphasis?: 'primary' | 'secondary';
		class?: string;
	} = $props();
</script>

{#if emphasis === 'primary'}
	<div class={cn('border-primary/40 bg-card space-y-2 rounded-lg border p-3', className)}>
		<div class="flex flex-wrap items-center justify-between gap-2">
			<p class="text-sm font-semibold">{label}</p>
			<CopyButton {value} label={`Copy ${label}`} text="Copy record" />
		</div>
		<pre
			class="bg-muted/50 selection:bg-primary/20 max-h-48 overflow-auto rounded-md border border-border px-3 py-2 font-mono text-sm leading-relaxed break-all whitespace-pre-wrap">{value}</pre>
	</div>
{:else}
	<div class={cn('space-y-1', className)}>
		<div class="flex items-center justify-between gap-2">
			<p class={cn('text-muted-foreground font-medium', emphasis === 'secondary' ? 'text-[11px]' : 'text-xs')}>
				{label}
			</p>
			<CopyButton {value} label={`Copy ${label}`} />
		</div>
		<pre
			class={cn(
				'bg-muted/50 selection:bg-primary/20 max-h-48 overflow-auto rounded-md border border-border font-mono leading-relaxed break-all whitespace-pre-wrap',
				emphasis === 'secondary' ? 'text-muted-foreground px-2.5 py-1.5 text-[11px]' : 'px-3 py-2 text-xs'
			)}>{value}</pre>
	</div>
{/if}
