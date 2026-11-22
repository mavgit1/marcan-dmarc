<script lang="ts">
	import { apiErrorMessage } from '#lib/http.ts';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { refresh } from '#lib/state/refresh.svelte.ts';
	import { cn } from '#lib/utils.ts';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	/**
	 * A failed refresh of data that is still on screen: one quiet line above it, so the last
	 * good figures stay readable. (Nothing loaded at all is `ErrorState`'s job.)
	 */
	let { error, class: className }: { error: unknown; class?: string } = $props();
</script>

<div
	class={cn('text-muted-foreground mb-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs', className)}
	role="status"
>
	<TriangleAlertIcon class="text-status-warn-fg size-3.5 shrink-0" />
	<span>Update failed, showing the last loaded data. {apiErrorMessage(error)}</span>
	<Button variant="ghost" size="xs" class="h-6 px-1.5 text-xs" onclick={() => refresh.refresh()}>Try again</Button>
</div>
