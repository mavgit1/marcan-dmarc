<script lang="ts">
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { meta } from '$lib/state/meta.svelte';
	import type { WordedIssue } from '$lib/types/WordedIssue';

	/**
	 * A domain's open issues as badges, each in its colour with what it means as the tooltip, in
	 * the fixed order of `meta.issue_catalogue`, so a kind sits in the same place on every row
	 * (the server's worst-first order until `meta` has loaded). One component for the dashboard's
	 * attention list and the Domains page's attention filter, so both show the same issues the
	 * same way.
	 */
	let { issues }: { issues: WordedIssue[] } = $props();

	const catalogue = $derived(meta.current?.issue_catalogue ?? []);
	const meaning = $derived(new Map(catalogue.map((i) => [i.kind, i.meaning])));
	const position = $derived(new Map(catalogue.map((i, n) => [i.kind, n])));
	const ordered = $derived(
		position.size ? [...issues].sort((a, b) => (position.get(a.kind) ?? 0) - (position.get(b.kind) ?? 0)) : issues
	);
	$effect(() => {
		meta.load().catch(() => {});
	});
</script>

<div class="flex flex-wrap gap-1.5">
	{#each ordered as issue (issue.kind)}
		<span title={meaning.get(issue.kind)}>
			<StatusBadge tone={issue.status}>{issue.label}</StatusBadge>
		</span>
	{/each}
</div>
