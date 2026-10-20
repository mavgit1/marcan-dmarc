<script lang="ts" module>
	/** The last held navigation a save bar scrolled into view for: one bar per hold scrolls. */
	let scrolledFor = 0;
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import type { Action } from '../action.svelte';
	import BusyButton from '../busy-button.svelte';
	import { unsavedSettings } from './settings-form.svelte';

	/**
	 * The foot of every Settings page: what is unsaved, Discard and Save, sticky at the bottom of
	 * the page. Each page is one form saved here. Save submits the surrounding form. It shows only
	 * while there is something to save (and while a save runs, failed, or just said how it went,
	 * for `SAVED_SHOWN_MS`); with nothing unsaved the page ends with its last section. While
	 * unsaved, a held navigation (another page) makes the bar show itself: it asks to save or
	 * discard, shakes once (unless reduced motion is asked for) and scrolls into view. `blocked`
	 * is why the unsaved page cannot be saved as it is: Save is disabled and the reason stands
	 * beside it.
	 */
	let {
		dirty,
		save,
		canSave = true,
		blocked = null,
		onDiscard
	}: {
		dirty: boolean;
		/** The page's save; its feedback is the status line. */
		save: Action;
		/** False while the draft is known to be invalid. */
		canSave?: boolean;
		/** Why Save cannot run ("Enter a server first."); `null` when it can. */
		blocked?: string | null;
		onDiscard: () => void;
	} = $props();

	const failed = $derived(save.feedback && !save.feedback.ok ? save.feedback.message : null);
	const saved = $derived(!dirty && save.feedback?.ok ? save.feedback.message : null);

	/** How long the bar stays after a save to say how it went ("Saved."), then it goes. */
	const SAVED_SHOWN_MS = 4000;
	let savedShown = $state(false);
	$effect(() => {
		if (!saved) return;
		savedShown = true;
		const id = setTimeout(() => (savedShown = false), SAVED_SHOWN_MS);
		return () => clearTimeout(id);
	});
	const visible = $derived(dirty || save.busy || !!failed || (!!saved && savedShown));

	let bar = $state<HTMLElement | null>(null);
	/** A navigation was held while this bar was unsaved; cleared once it is saved or discarded. */
	let asked = $state(false);
	let seen = untrack(() => unsavedSettings.held);
	$effect(() => {
		const held = unsavedSettings.held;
		if (held === seen) return;
		seen = held;
		if (!untrack(() => dirty) || !bar) return;
		asked = true;
		const el = bar;
		if (scrolledFor !== held) {
			scrolledFor = held;
			const box = el.getBoundingClientRect();
			if (box.top < 0 || box.bottom > window.innerHeight) el.scrollIntoView({ block: 'nearest' });
		}
		if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches)
			el.animate(
				[0, -6, 6, -4, 4, 0].map((x) => ({ transform: `translateX(${x}px)` })),
				{ duration: 360, easing: 'ease-out' }
			);
	});
	$effect(() => {
		if (!dirty) asked = false;
	});
</script>

{#if visible}
	<div
		bind:this={bar}
		class="bg-card/95 ring-foreground/10 sticky bottom-2 z-10 flex flex-wrap items-center justify-end gap-2 rounded-xl px-4 py-2 shadow-sm ring-1 backdrop-blur-sm"
	>
		<p class="mr-auto min-w-0 text-xs" role={failed ? 'alert' : 'status'}>
			{#if failed}
				<span class="text-status-fail-fg break-words">{failed}</span>
			{:else if dirty && asked}
				<span class="text-status-warn-fg font-medium">Save or discard the changes first</span>
			{:else if dirty && blocked}
				<span class="text-muted-foreground">{blocked}</span>
			{:else if dirty}
				<span class="text-muted-foreground">Unsaved changes</span>
			{:else if saved}
				<span class="text-status-pass-fg inline-flex items-center gap-1.5">
					<CircleCheckIcon class="size-3.5 shrink-0" />
					{saved}
				</span>
			{/if}
		</p>
		{#if dirty}
			<Button type="button" variant="ghost" size="sm" onclick={onDiscard} disabled={save.busy}>Discard</Button>
		{/if}
		<BusyButton type="submit" size="sm" busy={save.busy} busyLabel="Saving…" disabled={!dirty || !canSave || !!blocked}>
			Save
		</BusyButton>
	</div>
{/if}
