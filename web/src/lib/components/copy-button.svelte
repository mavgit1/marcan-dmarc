<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import { Button } from '$lib/components/ui/button/index.js';
	import { cn } from '$lib/utils.js';

	/**
	 * Copy an IP, DNS record, or other identifier to the clipboard.
	 * The button shows a check for 1.5s after a successful copy.
	 */
	let {
		value,
		label = 'Copy',
		text,
		class: className
	}: {
		value: string;
		/** Accessible name. Defaults to "Copy". */
		label?: string;
		/** A visible word beside the icon, as a primary button ("Copy record"); icon only without. */
		text?: string;
		class?: string;
	} = $props();

	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;

	async function copy() {
		try {
			await navigator.clipboard.writeText(value);
			copied = true;
			if (timer) clearTimeout(timer);
			timer = setTimeout(() => {
				copied = false;
			}, 1500);
		} catch {
			copied = false;
		}
	}
</script>

{#if text}
	<Button size="sm" class={className} aria-label={copied ? 'Copied' : label} onclick={copy}>
		{#if copied}
			<CheckIcon class="size-3.5" />Copied
		{:else}
			<CopyIcon class="size-3.5" />{text}
		{/if}
	</Button>
{:else}
	<Button
		variant="ghost"
		size="icon-xs"
		class={cn('text-muted-foreground', className)}
		aria-label={copied ? 'Copied' : label}
		title={copied ? 'Copied' : label}
		onclick={copy}
	>
		{#if copied}
			<CheckIcon class="size-3.5 text-status-pass-fg" />
		{:else}
			<CopyIcon class="size-3.5" />
		{/if}
	</Button>
{/if}
