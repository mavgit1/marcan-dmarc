<script lang="ts">
	import { DOCS_URL } from '$lib/nav';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';

	/** A Guide block's link to a docs page (`path` under the docs site), opening in a new tab. */
	let { path, label = 'Docs' }: { path: string; label?: string } = $props();
</script>

<a
	href="{DOCS_URL}{path}"
	target="_blank"
	rel="noopener"
	class="text-muted-foreground hover:text-foreground inline-flex items-center gap-1 text-xs underline-offset-2 hover:underline"
	>{label}<ExternalLinkIcon class="size-3" /></a
>
