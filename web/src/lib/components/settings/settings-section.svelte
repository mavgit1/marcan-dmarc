<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onFocusAnchor } from '$lib/state/anchor-focus.svelte';

	/**
	 * A section of a Settings page: the card surface, its title (left out where the page has one
	 * section and its title says it all) and its rows, separated by rules. `anchor` is the id a
	 * link opens (`SETTINGS_SECTIONS`); a link to it scrolls it into view. No folding, no switch
	 * and no save bar of its own; a feature that is off disables its rows through `SettingsPage`'s
	 * fieldset. The one fold is Alerts' Alert kinds (fifteen checkbox rows that would push Test far
	 * down), and its open state is that form's own flag, never a draft field.
	 */
	let {
		title,
		anchor,
		children
	}: {
		title?: string;
		/** The element id a link opens (a section of `SETTINGS_SECTIONS`). */
		anchor?: string;
		children: Snippet;
	} = $props();

	let element = $state<HTMLElement | null>(null);
	onFocusAnchor(
		() => anchor,
		() => element
	);
</script>

<section
	bind:this={element}
	id={anchor}
	aria-label={title}
	class="bg-card ring-foreground/10 scroll-mt-below-header rounded-xl ring-1"
>
	{#if title}
		<h3 class="border-b px-5 py-3 text-sm font-semibold">{title}</h3>
	{/if}
	<div class="divide-border min-w-0 divide-y">
		{@render children()}
	</div>
</section>
