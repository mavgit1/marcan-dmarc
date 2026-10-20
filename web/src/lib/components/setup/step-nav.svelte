<script lang="ts">
	import BusyButton from '$lib/components/busy-button.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import type { Snippet } from 'svelte';

	/**
	 * A step's Back and Continue, under its content. Continue is a submit button when the step
	 * is a form (`submit`), else it calls `onContinue`. `hint` stands beside it (why it is
	 * disabled, or that a running import keeps going).
	 */
	let {
		onBack,
		onContinue,
		label = 'Continue',
		submit = false,
		disabled = false,
		busy = false,
		hint
	}: {
		/** Omitted on the first step. */
		onBack?: () => void;
		onContinue?: () => void;
		label?: string;
		submit?: boolean;
		disabled?: boolean;
		busy?: boolean;
		hint?: string | Snippet;
	} = $props();
</script>

<div class="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
	{#if onBack}
		<Button type="button" variant="ghost" size="sm" class="gap-1.5" onclick={onBack}>
			<ArrowLeftIcon class="size-3.5" />
			Back
		</Button>
	{:else}
		<span></span>
	{/if}
	<div class="flex flex-wrap items-center justify-end gap-3">
		{#if hint}
			<span class="text-muted-foreground text-xs">
				{#if typeof hint === 'string'}{hint}{:else}{@render hint()}{/if}
			</span>
		{/if}
		{#if onContinue || submit}
			<BusyButton
				type={submit ? 'submit' : 'button'}
				size="sm"
				{busy}
				busyLabel="Saving…"
				{disabled}
				onclick={submit ? undefined : onContinue}
			>
				{label}
			</BusyButton>
		{/if}
	</div>
</div>
