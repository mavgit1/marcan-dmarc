<script lang="ts">
	import type { Snippet } from 'svelte';

	/**
	 * Literal DNS, record or tag text inside a sentence (`rua=mailto:…`, `p=reject`): monospace on
	 * a muted chip, a little smaller than the text around it, broken anywhere so a long record
	 * never widens the page.
	 */
	let { children }: { children: Snippet } = $props();
</script>

<code class="bg-muted rounded px-1 py-0.5 font-mono text-[0.85em] [overflow-wrap:anywhere]">{@render children()}</code>
