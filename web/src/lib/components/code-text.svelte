<script lang="ts">
	import { codeSpans } from '$lib/text';
	import InlineCode from '$lib/components/inline-code.svelte';

	/**
	 * A server sentence whose backtick spans are literal record text (DNS findings and their
	 * suggestions): those spans render as `InlineCode`, the rest as plain text. The server keeps
	 * the backticks so CSV exports read the same as before.
	 */
	let { text }: { text: string } = $props();
</script>

<!-- eslint-disable-next-line svelte/require-each-key -- parts are positional and never reorder -->
{#each codeSpans(text) as part}{#if part.code}<InlineCode>{part.text}</InlineCode>{:else}{part.text}{/if}{/each}
