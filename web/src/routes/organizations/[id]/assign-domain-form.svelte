<script lang="ts">
	import { updateDomain } from '$lib/api';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { refresh } from '$lib/state/refresh.svelte';
	import { scopeOptions } from '$lib/state/scope.svelte';

	/** The header's inline "Assign domain": move a monitored domain of another organization here. */
	let { organizationId, onClose }: { organizationId: number; onClose: () => void } = $props();

	/** Monitored domains of other organizations, from the shared scope list. */
	const assignable = $derived(
		(scopeOptions.current?.domains ?? []).filter((d) => d.organization_id !== organizationId)
	);
	let picked = $state<number | null>(null);
	const selectedId = $derived(assignable.some((d) => d.id === picked) ? picked : (assignable[0]?.id ?? null));
	let loadError = $state<string | null>(null);
	const loading = $derived(scopeOptions.current === null && loadError === null);
	const assign = new Action();

	$effect(() => {
		scopeOptions.load().catch(() => {
			loadError = 'Failed to load domains';
		});
	});

	function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		if (!selectedId || assign.busy) return;
		void assign.run(async () => {
			await updateDomain(selectedId, { organization_id: organizationId });
			refresh.refresh();
			onClose();
		});
	}

	const failed = $derived(loadError ?? (assign.feedback && !assign.feedback.ok ? assign.feedback.message : null));
</script>

<form class="flex flex-wrap items-center gap-1.5" onsubmit={onSubmit}>
	{#if loading}
		<Skeleton class="h-8 w-40" />
	{:else if loadError}
		<!-- the error line below says why -->
	{:else if assignable.length === 0}
		<span class="text-xs text-muted-foreground">All domains already assigned</span>
	{:else}
		<select
			value={selectedId}
			onchange={(e) => (picked = Number(e.currentTarget.value))}
			class="h-8 rounded-md border border-input bg-background px-2 text-xs"
			aria-label="Select existing domain"
		>
			{#each assignable as d (d.id)}
				<option value={d.id}>{d.domain}</option>
			{/each}
		</select>
		<BusyButton
			type="submit"
			size="sm"
			class="h-8 text-xs"
			busy={assign.busy}
			busyLabel="Assigning…"
			disabled={!selectedId}
		>
			Assign to organization
		</BusyButton>
	{/if}
	<Button type="button" variant="ghost" size="sm" class="h-8 text-xs" disabled={assign.busy} onclick={onClose}>
		Cancel
	</Button>
	{#if failed}
		<p class="text-destructive w-full text-xs font-medium" role="alert">{failed}</p>
	{/if}
</form>
