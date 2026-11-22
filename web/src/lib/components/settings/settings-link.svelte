<script lang="ts">
	import { settingsAnchor, settingsHref, settingsTitle, type SettingsTarget } from '#lib/settings-nav.ts';
	import { cn } from '#lib/utils.ts';
	import type { Snippet } from 'svelte';
	import { focusAnchor } from '#lib/state/anchor-focus.svelte.ts';

	/**
	 * A link to a Settings page or to a section on one (its title, or `children`): opens the page
	 * and scrolls to the section, also when the link is followed a second time.
	 */
	let { to, class: className, children }: { to: SettingsTarget; class?: string; children?: Snippet } = $props();
</script>

<a
	href={settingsHref(to)}
	class={cn('text-foreground underline underline-offset-2', className)}
	onclick={() => {
		const anchor = settingsAnchor(to);
		if (anchor) focusAnchor(anchor);
	}}
>
	{#if children}{@render children()}{:else}{settingsTitle(to)}{/if}
</a>
