<script lang="ts">
	import { resolve } from '$app/paths';
	import { updateOrganization } from '$lib/api';
	import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
	import { Action } from '$lib/components/action.svelte';
	import InlineCode from '$lib/components/inline-code.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { untrack } from 'svelte';
	import StepNav from './step-nav.svelte';

	/** The organization new domains land in: renamed here, or kept as "Default". */
	let {
		organization,
		readonly = false,
		onSaved,
		onNext
	}: {
		organization: OrganizationSummary;
		readonly?: boolean;
		/** Saved (even unchanged): the step is passed. */
		onSaved: (organization: OrganizationSummary) => void;
		onNext: () => void;
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

<!-- The first step opens with how reports reach Marcan; the later steps go into each part. -->
<section aria-labelledby="setup-how-reports" class="mb-4 space-y-1">
	<h3 id="setup-how-reports" class="text-sm font-medium">How reports reach Marcan</h3>
	<p class="text-muted-foreground text-xs leading-relaxed">
		Each of your domains publishes a DMARC record whose <InlineCode>rua=</InlineCode> points at the report mailbox, directly
		or through an address that forwards to it. If that mailbox is on another domain, that domain publishes a
		<InlineCode>_report._dmarc</InlineCode> authorization record for each of yours. Receivers then send one report a day per
		domain, and Marcan reads the mailbox without changing anything in it.
	</p>
	<p class="text-muted-foreground text-xs leading-relaxed">
		Only SPF or DKIM aligned with the From domain counts, and enforcement goes <InlineCode>p=none</InlineCode>,
		<InlineCode>quarantine</InlineCode>, <InlineCode>reject</InlineCode>; readiness tells you when a domain is ready for
		the next step.
	</p>
</section>

{#if readonly}
	<div class="space-y-4">
		<p class="text-muted-foreground text-xs leading-relaxed">
			New domains land in <span class="text-foreground font-medium">{organization.name}</span>. Rename it or add one
			organization per client under
			<a href={resolve('/organizations')} class="text-foreground underline underline-offset-2">Organizations</a>.
		</p>
		<StepNav label="Next" onContinue={onNext} />
	</div>
{:else}
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
{/if}
