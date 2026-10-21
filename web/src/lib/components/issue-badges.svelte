<script lang="ts">
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { meta } from '$lib/state/meta.svelte';
	import type { WordedIssue } from '$lib/types/WordedIssue';

	/**
	 * A domain's open issues as badges, worst first as the server lists them (`assessment.issues`),
	 * each in its colour with what it means as the tooltip (`meta.issue_catalogue`). One component
	 * for the dashboard's attention list and the Domains page's attention filter, so both show the
	 * same issues the same way.
	 */
	let { issues }: { issues: WordedIssue[] } = $props();

	const meaning = $derived(new Map(meta.current?.issue_catalogue.map((i) => [i.kind, i.meaning]) ?? []));
	$effect(() => {
		meta.load().catch(() => {});
	});
</script>

<div class="flex flex-wrap gap-1.5">
	{#each issues as issue (issue.kind)}
		<span title={meaning.get(issue.kind)}>
			<StatusBadge tone={issue.status}>{issue.label}</StatusBadge>
		</span>
	{/each}
</div>
