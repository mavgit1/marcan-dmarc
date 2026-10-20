<script lang="ts" generics="Id extends string">
	import { Button } from '$lib/components/ui/button/index.js';
	import { cn } from '$lib/utils.js';

	/** Underline tabs for a page's sections (the domain page), the active one kept in the URL (`?tab=`). */
	let {
		tabs,
		active,
		label,
		onSelect
	}: {
		tabs: readonly { id: Id; label: string }[];
		active: Id;
		label: string;
		onSelect: (id: Id) => void;
	} = $props();
</script>

<nav class="mb-6 flex flex-wrap gap-1 border-b border-border pb-px" aria-label={label}>
	{#each tabs as tab (tab.id)}
		<Button
			type="button"
			variant="ghost"
			size="sm"
			class={cn(
				'text-muted-foreground hover:text-foreground -mb-px rounded-none border-0 border-b-2 border-b-transparent px-3 hover:bg-transparent',
				active === tab.id && 'border-b-foreground text-foreground'
			)}
			aria-current={active === tab.id ? 'page' : undefined}
			onclick={() => onSelect(tab.id)}
		>
			{tab.label}
		</Button>
	{/each}
</nav>
