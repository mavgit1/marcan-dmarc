<script lang="ts">
	import { cn } from '$lib/utils.js';
	import type { Snippet } from 'svelte';

	let { class: className, children }: { class?: string; children: Snippet } = $props();
</script>

<h4 class={cn('text-xs font-semibold uppercase tracking-wider text-muted-foreground', className)}>
	{@render children()}
</h4>
