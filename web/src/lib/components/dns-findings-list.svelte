<script lang="ts">
	import { formatPoints, SEVERITY_LABEL, type ListedFinding } from '$lib/dns';
	import { SEVERITY_TONE, statusRow, type StatusTone } from '$lib/tokens';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { cn } from '$lib/utils.js';

	/**
	 * Findings list for DNS check panels and the domain overview.
	 *
	 * Always shows the suggestion with the finding, so every item is actionable in place.
	 *
	 * Never key the each-block: finding codes repeat across selectors.
	 */
	let {
		findings,
		itemTone,
		showDeductions = true,
		class: className
	}: {
		findings: readonly ListedFinding[];
		/** Tints every item, for a list the caller has already filtered to one severity. */
		itemTone?: StatusTone;
		showDeductions?: boolean;
		class?: string;
	} = $props();
</script>

{#if findings.length > 0}
	<ul class={cn('space-y-2', className)}>
		<!-- eslint-disable-next-line svelte/require-each-key -- finding codes repeat across selectors -->
		{#each findings as finding}
			<li class={cn('rounded-md border border-border px-3 py-2', itemTone ? statusRow[itemTone] : 'bg-card')}>
				<div class="mb-1 flex flex-wrap items-center gap-2">
					<!-- The finding code is internal (CSV, support): tooltip only. -->
					<span title={finding.code}>
						<StatusBadge tone={SEVERITY_TONE[finding.severity]}>
							{SEVERITY_LABEL[finding.severity]}
						</StatusBadge>
					</span>
					{#if showDeductions}
						<span class="text-muted-foreground text-xs font-semibold tabular-nums" title="Score points lost">
							{formatPoints(finding.points)}
						</span>
					{/if}
				</div>
				<p class="text-sm">{finding.message}</p>
				{#if finding.suggestion}
					<p class="text-muted-foreground mt-1 text-xs">{finding.suggestion}</p>
				{/if}
			</li>
		{/each}
	</ul>
{/if}
