<script lang="ts">
	import type { IncludeNode } from '$lib/types/IncludeNode';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import { spfErrorExplanation, spfSubtreeLookups } from '$lib/dns';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { statusRow } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import SpfIncludeTree from './spf-include-tree.svelte';

	/**
	 * Recursive SPF include/redirect tree. Shows own lookup cost and subtree total
	 * so operators can see which include eats the budget.
	 * Do not key the children each-block: domain names can repeat across branches.
	 */
	let {
		node,
		depth = 0
	}: {
		node: IncludeNode;
		depth?: number;
	} = $props();

	const subtree = $derived(spfSubtreeLookups(node));
	const explanation = $derived(spfErrorExplanation(node.error));
	const isError = $derived(Boolean(node.error));
</script>

<div
	class={cn(
		'rounded-md border px-3 py-2',
		isError ? cn(statusRow.fail, 'border-status-fail-border') : 'border-border bg-background'
	)}
	style:margin-left={depth > 0 ? `${Math.min(depth, 8) * 0.75}rem` : undefined}
>
	<div class="flex flex-wrap items-baseline justify-between gap-2">
		<div class="min-w-0">
			<p class="font-mono text-sm font-medium break-all">{node.domain}</p>
			{#if isError}
				<StatusBadge tone="fail" class="mt-1">Error</StatusBadge>
			{/if}
		</div>
		<div class="text-muted-foreground flex flex-wrap gap-3 text-xs tabular-nums">
			<span title="Lookups charged to this node">own {node.lookups}</span>
			<span title="Own lookups plus every descendant">subtree {subtree}</span>
		</div>
	</div>

	{#if explanation}
		<p class="text-status-fail-fg mt-2 text-sm">{explanation}</p>
	{/if}

	{#if node.record}
		<div class="mt-2">
			<DnsRawRecord value={node.record} label="SPF record" />
		</div>
	{:else if !isError}
		<p class="text-muted-foreground mt-2 text-xs">No SPF record string on this node.</p>
	{/if}

	{#if node.children.length > 0}
		<div class="mt-3 space-y-2">
			<!-- eslint-disable-next-line svelte/require-each-key -- a record can include the same name twice -->
			{#each node.children as child}
				<SpfIncludeTree node={child} depth={depth + 1} />
			{/each}
		</div>
	{/if}
</div>
