<script lang="ts">
	import type { Health } from '$lib/types/Health';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { HEALTH_LABEL } from '$lib/health';

	/** Is delivered mail authentic? The server's health verdict, worded. */
	let { health, class: className }: { health: Health; class?: string } = $props();
</script>

<StatusBadge tone={health.status} class={className}>{HEALTH_LABEL[health.state]}</StatusBadge>
