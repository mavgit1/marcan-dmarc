<script lang="ts">
	import { Badge } from '#lib/components/ui/badge/index.ts';
	import { statusSurface, type StatusTone } from '#lib/tokens.ts';
	import { cn } from '#lib/utils.ts';
	import type { Snippet } from 'svelte';

	/** Outline badge tinted with a status tone. Keep a text label: colour alone is not a status. */
	let { tone, class: className, children }: { tone: StatusTone; class?: string; children: Snippet } = $props();
</script>

<Badge variant="outline" class={cn(className, statusSurface[tone])}>{@render children()}</Badge>
