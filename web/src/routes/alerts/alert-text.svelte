<script lang="ts">
	import StatusBadge from '$lib/components/status-badge.svelte';
	import type { AlertItem } from '$lib/types/AlertItem';

	/**
	 * One event: its colour as a badge (the row's verdict) and its sentence, both worded by the
	 * server (`severity_label`, `text`: the alert email's words).
	 */
	let { item }: { item: AlertItem } = $props();
</script>

<span class="flex min-w-0 items-baseline gap-2">
	<StatusBadge tone={item.event.severity} class="shrink-0">{item.severity_label}</StatusBadge>
	<span class="min-w-0">{item.text}</span>
</span>
