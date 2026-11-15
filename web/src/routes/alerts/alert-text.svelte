<script lang="ts">
	import { resolve } from '$app/paths';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import type { AlertItem } from '$lib/types/AlertItem';

	/**
	 * One event: its colour as a badge (the row's verdict) and its sentence, both worded by the
	 * server (`severity_label`, `text`: the alert email's words); an event about a sender links the
	 * sentence's own closing words (`sender_link_words`) to its sender page, and a newer release
	 * links its release notes, as the alert email does. The alert list sets `badge` false: it
	 * shows the badge in a column of its own.
	 */
	let { item, badge = true }: { item: AlertItem; badge?: boolean } = $props();

	/** The sentence before the linked words, and the words; `null` when nothing links in it. */
	const senderLink = $derived.by(() => {
		const words = item.sender_link_words;
		if (!item.sender_ip || !words || !item.text.endsWith(`${words}.`)) return null;
		return { head: item.text.slice(0, -(words.length + 1)), words };
	});
</script>

<span class="flex min-w-0 items-baseline gap-2">
	{#if badge}
		<StatusBadge tone={item.event.severity} class="shrink-0">{item.severity_label}</StatusBadge>
	{/if}
	<span class="min-w-0"
		>{#if senderLink && item.sender_ip}{senderLink.head}<a
				href={resolve(`/senders/${encodeURIComponent(item.sender_ip)}`)}
				class="underline underline-offset-2">{senderLink.words}</a
			>.{:else}{item.text}{#if item.sender_ip}{' '}<a
					href={resolve(`/senders/${encodeURIComponent(item.sender_ip)}`)}
					class="underline underline-offset-2">Sender page</a
				>{:else if item.release_notes}{' '}<a
					href={item.release_notes}
					target="_blank"
					rel="noopener"
					class="underline underline-offset-2">Release notes</a
				>{/if}{/if}</span
	>
</span>
