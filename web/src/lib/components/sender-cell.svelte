<script lang="ts">
	import CopyButton from '$lib/components/copy-button.svelte';
	import SenderName from '$lib/components/sender-name.svelte';
	import type { SenderIdentity } from '$lib/types/SenderIdentity';
	import type { Snippet } from 'svelte';

	/**
	 * The one source-IP table cell: the IP (a link with `href`) with its copy control, the
	 * sender's caption under it when `identity` is given, and any extra line as children.
	 */
	let {
		ip,
		identity,
		href,
		children
	}: {
		ip: string;
		/** Omit to show the IP alone (the sender list has its own name column). */
		identity?: SenderIdentity | null;
		href?: string;
		children?: Snippet;
	} = $props();
</script>

<div class="flex min-w-0 flex-col">
	<div class="flex items-center gap-1">
		{#if href}
			<a {href} class="font-mono text-xs hover:underline">{ip}</a>
		{:else}
			<span class="font-mono text-xs">{ip}</span>
		{/if}
		<CopyButton value={ip} label="Copy source IP" />
	</div>
	{#if identity !== undefined}
		<SenderName {identity} class="max-w-[200px] truncate text-xs text-muted-foreground" />
	{/if}
	{@render children?.()}
</div>
