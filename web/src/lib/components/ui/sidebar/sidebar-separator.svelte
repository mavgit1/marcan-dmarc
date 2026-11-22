<script lang="ts">
	import { Separator } from '#lib/components/ui/separator/index.ts';
	import { cn } from '#lib/utils.ts';
	import type { ComponentProps } from 'svelte';

	let { ref = $bindable(null), class: className, ...restProps }: ComponentProps<typeof Separator> = $props();
</script>

<Separator
	bind:ref
	data-slot="sidebar-separator"
	data-sidebar="separator"
	class={cn('mx-2 bg-sidebar-border w-auto', className)}
	{...restProps}
/>
