<script lang="ts">
	import { updateOrganization } from '$lib/api';
	import { Action } from '$lib/components/action.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { refresh } from '$lib/state/refresh.svelte';
	import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
	import { untrack } from 'svelte';

	/** The header's inline rename; an unchanged name just closes it. */
	let { organization, onClose }: { organization: OrganizationSummary; onClose: () => void } = $props();

	let name = $state(untrack(() => organization.name));
	const rename = new Action();

	function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		const trimmed = name.trim();
		if (!trimmed || rename.busy) return;
		if (trimmed === organization.name) return onClose();
		void rename.run(async () => {
			await updateOrganization(organization.id, { name: trimmed });
			refresh.refresh();
			onClose();
		});
	}
</script>

<form class="flex flex-wrap items-center gap-1.5" onsubmit={onSubmit}>
	<Input bind:value={name} required maxlength={120} aria-label="Organization name" class="h-8 w-48 text-xs" />
	<BusyButton
		type="submit"
		size="sm"
		class="h-8 text-xs"
		busy={rename.busy}
		busyLabel="Saving…"
		disabled={!name.trim()}
	>
		Save name
	</BusyButton>
	<Button type="button" variant="ghost" size="sm" class="h-8 text-xs" disabled={rename.busy} onclick={onClose}>
		Cancel
	</Button>
	{#if rename.feedback && !rename.feedback.ok}
		<p class="text-destructive w-full text-xs font-medium" role="alert">{rename.feedback.message}</p>
	{/if}
</form>
