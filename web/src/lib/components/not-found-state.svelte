<script lang="ts">
	import EmptyState from '$lib/components/empty-state.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import FileQuestionIcon from '@lucide/svelte/icons/file-question';

	/**
	 * The one "this does not exist" page body (report, sender, domain, organization): what is
	 * missing and a way back to its list. Real failures use `ErrorState` with a retry.
	 */
	let {
		headline,
		description,
		backHref,
		backLabel
	}: {
		headline: string;
		description?: string;
		/** Already resolved (`resolve('/reports')`). */
		backHref: string;
		backLabel: string;
	} = $props();
</script>

<EmptyState icon={FileQuestionIcon} {headline} {description}>
	{#snippet action()}
		<Button variant="outline" size="sm" href={backHref}>{backLabel}</Button>
	{/snippet}
</EmptyState>
