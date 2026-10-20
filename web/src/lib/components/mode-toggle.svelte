<script lang="ts">
	import MoonIcon from '@lucide/svelte/icons/moon';
	import SunIcon from '@lucide/svelte/icons/sun';
	import MonitorIcon from '@lucide/svelte/icons/monitor';
	import { resetMode, setMode, userPrefersMode } from 'mode-watcher';
	import { buttonVariants } from '$lib/components/ui/button/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import { cn } from '$lib/utils.js';

	const preference = $derived(userPrefersMode.current);
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger
		class={cn(buttonVariants({ variant: 'ghost', size: 'icon-sm' }), 'relative')}
		aria-label="Colour theme"
		title="Colour theme"
	>
		<SunIcon class="size-4 scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
		<MoonIcon class="absolute size-4 scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" />
		<span class="sr-only">Colour theme</span>
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end">
		<DropdownMenu.Item onclick={() => setMode('light')}>
			<SunIcon class="size-4" />
			Light
			{#if preference === 'light'}
				<span class="text-muted-foreground ms-auto text-xs">Active</span>
			{/if}
		</DropdownMenu.Item>
		<DropdownMenu.Item onclick={() => setMode('dark')}>
			<MoonIcon class="size-4" />
			Dark
			{#if preference === 'dark'}
				<span class="text-muted-foreground ms-auto text-xs">Active</span>
			{/if}
		</DropdownMenu.Item>
		<DropdownMenu.Item onclick={() => resetMode()}>
			<MonitorIcon class="size-4" />
			System
			{#if preference === 'system'}
				<span class="text-muted-foreground ms-auto text-xs">Active</span>
			{/if}
		</DropdownMenu.Item>
	</DropdownMenu.Content>
</DropdownMenu.Root>
