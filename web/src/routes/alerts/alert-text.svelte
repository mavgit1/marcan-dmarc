<script lang="ts">
	import { resolve } from '$app/paths';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import type { AlertItem } from '$lib/types/AlertItem';

	/**
	 * One event: its colour as a badge (the row's verdict) and its sentence, both worded by the
	 * server (`severity_label`, `text`: the alert email's words); an event about a sender links to
	 * its sender page, and a newer release to its release notes, as the alert email does.
	 */
	let { item }: { item: AlertItem } = $props();
</script>

<span class="flex min-w-0 items-baseline gap-2">
	<StatusBadge tone={item.event.severity} class="shrink-0">{item.severity_label}</StatusBadge>
	<span class="min-w-0"
		>{item.text}{#if item.sender_ip}{' '}<a
				href={resolve(`/senders/${encodeURIComponent(item.sender_ip)}`)}
				class="underline underline-offset-2">Sender page</a
			>{:else if item.release_notes}{' '}<a
				href={item.release_notes}
				target="_blank"
				rel="noopener"
				class="underline underline-offset-2">Release notes</a
			>{/if}</span
	>
</span>
