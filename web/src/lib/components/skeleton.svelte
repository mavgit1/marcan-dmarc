<script lang="ts">
	import { Skeleton as UiSkeleton } from '$lib/components/ui/skeleton/index.js';
	import { cn } from '$lib/utils.js';

	/**
	 * Loading placeholder. Prefer this over the ui/ primitive so pages share one import path.
	 *
	 * - `text` — a line of copy
	 * - `block` — a card-sized rectangle
	 * - `avatar` — a square mark
	 */
	let {
		variant = 'text',
		class: className
	}: {
		variant?: 'text' | 'block' | 'avatar';
		class?: string;
	} = $props();
</script>

<UiSkeleton
	class={cn(
		variant === 'text' && 'h-4 w-full',
		variant === 'block' && 'h-24 w-full',
		variant === 'avatar' && 'size-8 rounded-md',
		className
	)}
/>
