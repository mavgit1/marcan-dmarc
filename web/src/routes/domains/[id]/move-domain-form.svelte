<script lang="ts">
	import { updateDomain } from '$lib/api';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { refresh } from '$lib/state/refresh.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';

	/** The header's inline "Move to organization" picker. */
	let {
		domainId,
		organizationId,
		onClose
	}: {
		domainId: number;
		/** The domain's current organization (listed, not selectable). */
		organizationId: number;
		onClose: () => void;
	} = $props();

	/** Names from the shared scope list; it stays current across refreshes. */
	const organizations = $derived(scopeOptions.current?.organizations ?? []);
	let target = $state<number | null>(null);
	let loadError = $state<string | null>(null);
	const move = new Action();

	$effect(() => {
		scopeOptions.load().then(
			(scope) => {
				if (target !== null) return;
				const other = scope.organizations.find((c) => c.id !== organizationId);
				target = other ? other.id : null;
			},
			() => {
				loadError = 'Failed to load organizations';
			}
		);
	});

	function onMove() {
		if (!target || move.busy) return;
		const to = target;
		void move.run(async () => {
			await updateDomain(domainId, { organization_id: to });
			refresh.refresh();
			onClose();
		});
	}

	const failed = $derived(loadError ?? (move.feedback && !move.feedback.ok ? move.feedback.message : null));
</script>

<div class="flex flex-wrap items-center gap-1.5 rounded-md border border-input bg-background p-1 text-xs">
	<span class="text-muted-foreground pl-1">Move to:</span>
	<select
		bind:value={target}
		class="h-7 max-w-full min-w-0 rounded border border-input bg-background px-1.5 text-xs text-foreground"
		aria-label="Target organization"
	>
		{#each organizations as c (c.id)}
			<option value={c.id} disabled={c.id === organizationId}>
				{c.name}
				{c.id === organizationId ? '(current)' : ''}
			</option>
		{/each}
	</select>
	<BusyButton
		size="sm"
		class="h-7 px-2 text-xs"
		busy={move.busy}
		busyLabel="Moving…"
		disabled={!target || target === organizationId}
		onclick={onMove}
	>
		Confirm
	</BusyButton>
	<Button variant="ghost" size="sm" class="h-7 px-1.5 text-xs" disabled={move.busy} onclick={onClose}>Cancel</Button>
	{#if failed}
		<p class="text-destructive w-full px-1 text-xs font-medium" role="alert">{failed}</p>
	{/if}
</div>
