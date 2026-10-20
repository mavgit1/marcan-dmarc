<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import { apiErrorMessage } from '$lib/http';
	import { Button } from '$lib/components/ui/button/index.js';
	import { cn } from '$lib/utils.js';

	/**
	 * Inline error with an optional retry action. Pass what was thrown as `error` (it is
	 * worded by `apiErrorMessage`), or a ready sentence as `message`. Never pass
	 * `error.message`: it is ky's raw request text.
	 */
	let {
		title = 'Something went wrong',
		error,
		message,
		onRetry,
		retryLabel = 'Try again',
		class: className
	}: {
		title?: string;
		error?: unknown;
		message?: string;
		onRetry?: () => void;
		retryLabel?: string;
		class?: string;
	} = $props();

	const text = $derived(message ?? (error === undefined ? '' : apiErrorMessage(error)));
</script>

<div
	class={cn(
		'border-status-fail-border bg-status-fail-bg/40 flex flex-col items-start gap-3 rounded-lg border px-4 py-4',
		className
	)}
	role="alert"
>
	<div class="flex items-start gap-2.5">
		<CircleAlertIcon class="text-status-fail-fg mt-0.5 size-4 shrink-0" />
		<div class="space-y-1">
			<p class="text-sm font-medium">{title}</p>
			{#if text}
				<p class="text-muted-foreground text-sm">{text}</p>
			{/if}
		</div>
	</div>
	{#if onRetry}
		<Button variant="outline" size="sm" onclick={onRetry}>{retryLabel}</Button>
	{/if}
</div>
