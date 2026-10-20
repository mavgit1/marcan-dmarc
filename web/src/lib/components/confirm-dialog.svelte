<script lang="ts">
	import type { Action } from './action.svelte';
	import BusyButton from './busy-button.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import type { Snippet } from 'svelte';

	/**
	 * The inline confirmation of a destructive action (delete an organization, ignore a
	 * domain): what happens as `children`, Cancel, and the destructive button driven by
	 * `action` (busy while it runs; a failure shows under the text). Never `window.confirm`:
	 * embedded browsers answer it unseen.
	 */
	let {
		title,
		confirmLabel,
		busyLabel,
		action,
		onConfirm,
		onCancel,
		children
	}: {
		title: string;
		confirmLabel: string;
		busyLabel: string;
		action: Action;
		onConfirm: () => void;
		onCancel: () => void;
		children: Snippet;
	} = $props();

	const id = $props.id();
	const failed = $derived(action.feedback && !action.feedback.ok ? action.feedback.message : null);
</script>

<div
	class="border-status-fail-border bg-status-fail-bg/40 mb-6 flex flex-wrap items-center justify-between gap-3 rounded-lg border px-4 py-3"
	role="alertdialog"
	aria-labelledby="{id}-title"
>
	<div class="space-y-0.5 text-sm">
		<p id="{id}-title" class="font-medium">{title}</p>
		{@render children()}
		{#if failed}
			<p class="text-status-fail-fg text-xs" role="alert">{failed}</p>
		{/if}
	</div>
	<div class="flex items-center gap-2">
		<Button variant="ghost" size="sm" disabled={action.busy} onclick={onCancel}>Cancel</Button>
		<BusyButton variant="destructive" size="sm" busy={action.busy} {busyLabel} onclick={onConfirm}>
			{confirmLabel}
		</BusyButton>
	</div>
</div>
