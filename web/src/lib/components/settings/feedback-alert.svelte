<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import type { Feedback } from '../action.svelte';

	/** Pass or fail banner for a settings action. `failTitle` puts a heading over a failure. */
	let {
		feedback,
		failTitle,
		okFallback = ''
	}: { feedback: Feedback | null; failTitle?: string; okFallback?: string } = $props();
</script>

{#if feedback?.ok}
	<div
		class="border-status-pass-border bg-status-pass-bg text-status-pass-fg flex items-center gap-2 rounded-md border p-3 text-xs"
		role="status"
	>
		<CircleCheckIcon class="size-4 shrink-0" />
		<span>{feedback.message || okFallback}</span>
	</div>
{:else if feedback}
	<div
		class="border-status-fail-border bg-status-fail-bg text-status-fail-fg flex items-start gap-2 rounded-md border p-3 text-xs"
		role="alert"
	>
		<CircleAlertIcon class="mt-0.5 size-4 shrink-0" />
		{#if failTitle}
			<div class="min-w-0 flex-1">
				<p class="font-medium">{failTitle}</p>
				<p class="mt-0.5 break-words text-[11px]">{feedback.message}</p>
			</div>
		{:else}
			<span class="min-w-0 flex-1">{feedback.message}</span>
		{/if}
	</div>
{/if}
