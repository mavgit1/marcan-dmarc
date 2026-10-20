<script lang="ts">
	import { findingsWorstFirst, type ListedFinding } from '$lib/dns';
	import DnsFindingsList from '$lib/components/dns-findings-list.svelte';
	import DetailHeading from './detail-heading.svelte';

	/** What is wrong, worst first: the top of every inspection drawer. */
	let {
		title,
		findings,
		showDeductions = true
	}: { title: string; findings: readonly ListedFinding[]; showDeductions?: boolean } = $props();

	const sorted = $derived(findingsWorstFirst(findings));
</script>

{#if sorted.length > 0}
	<div>
		<DetailHeading class="mb-2">{title}</DetailHeading>
		<DnsFindingsList findings={sorted} {showDeductions} />
	</div>
{/if}
