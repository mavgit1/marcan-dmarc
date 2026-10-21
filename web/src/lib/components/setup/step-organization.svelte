<script lang="ts">
	import { updateOrganization } from '$lib/api';
	import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
	import { Action } from '$lib/components/action.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { untrack } from 'svelte';
	import StepNav from './step-nav.svelte';

	/** The organization new domains land in: renamed here, or kept as "Default". */
	let {
		organization,
		onSaved
	}: {
		organization: OrganizationSummary;
		/** Saved (even unchanged): the step is passed. */
		onSaved: (organization: OrganizationSummary) => void;
	} = $props();

	let name = $state(untrack(() => organization.name));
	const save = new Action();

	function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		save.run(async () => {
			const trimmed = name.trim();
			const saved =
				trimmed === organization.name ? organization : await updateOrganization(organization.id, { name: trimmed });
			onSaved(saved);
		});
	}
</script>

<form class="space-y-4" onsubmit={onSubmit}>
	<p class="text-muted-foreground text-xs leading-relaxed">
		New domains land here. Rename it, or keep "Default" and add one organization per client later.
	</p>
	<div class="max-w-xs">
		<label for="setup-org-name" class="text-muted-foreground text-xs font-medium">Organization name</label>
		<Input id="setup-org-name" bind:value={name} required maxlength={120} autocomplete="organization" class="mt-1" />
	</div>
	<FeedbackAlert feedback={save.feedback} failTitle="Rename failed" />
	<StepNav submit busy={save.busy} disabled={!name.trim()} />
</form>
