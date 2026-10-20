<script lang="ts">
	import Skeleton from '$lib/components/skeleton.svelte';
	import { formatPoints, worstVerdict, type PillarRow, type Verdict } from '$lib/dns';
	import { statusBorderL, statusText, statusTextPlain } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import ChevronUpIcon from '@lucide/svelte/icons/chevron-up';

	/**
	 * One scorecard pillar: toggles its inspection drawer. No badge: the card's own verdict is
	 * its first row (`verdictLabel`). The left border shows the worst of every toned row and the
	 * findings; the points take the findings' colour, so a finding that is not a row still
	 * explains the border. The findings themselves are listed first in the drawer.
	 */
	let {
		label,
		lostPts,
		verdict,
		verdictLabel,
		findings,
		pending = false,
		active,
		onclick,
		rows = []
	}: {
		label: string;
		/** Score points this pillar costs; `null` for a pillar that is not scored, omitted while unknown. */
		lostPts?: number | null;
		/** The pillar's own verdict, shown as the first row. */
		verdict: Verdict;
		/** Row label for `verdict`. */
		verdictLabel: string;
		/** The pillar's `findingsVerdict`: colours the points and counts toward the border. */
		findings?: Verdict | null;
		/** The check has not answered yet: skeleton rows, no verdict. */
		pending?: boolean;
		active: boolean;
		onclick: () => void;
		rows?: PillarRow[];
	} = $props();

	const shownRows = $derived<PillarRow[]>([
		{ label: verdictLabel, value: verdict.label, tone: verdict.tone, mono: verdict.mono },
		...rows
	]);
	const tone = $derived(
		pending
			? 'neutral'
			: worstVerdict(...shownRows.map((r) => (r.tone ? { label: r.value, tone: r.tone } : null)), findings).tone
	);
	/** Points coloured by the findings that cost them; a pass (no issues) stays plain. */
	const pointsTone = $derived(findings && findings.tone !== 'pass' ? findings.tone : 'neutral');
</script>

<button
	type="button"
	class={cn(
		'flex flex-col justify-between rounded-lg border border-l-2 bg-card p-3.5 text-left transition-colors hover:bg-muted/20',
		active ? 'border-foreground/40 bg-muted/20' : 'border-border',
		statusBorderL[tone]
	)}
	{onclick}
	aria-expanded={active}
>
	<div>
		<div class="mb-2 flex items-center justify-between">
			<span class="text-muted-foreground text-xs font-medium">{label}</span>
			{#if lostPts === null}
				<span class="text-muted-foreground text-[11px]">Not scored</span>
			{:else if lostPts !== undefined && !pending}
				<span
					class={cn('text-xs font-semibold tabular-nums', statusText[pointsTone])}
					title={findings ? `Score points lost · ${findings.label}` : 'Score points lost'}
				>
					{formatPoints(lostPts)}
				</span>
			{/if}
		</div>
		{#if pending}
			<div class="space-y-1.5">
				<Skeleton class="h-3.5 w-full" />
				<Skeleton class="h-3.5 w-3/4" />
			</div>
		{:else}
			<div class="space-y-1 text-xs">
				{#each shownRows as row, i (row.label)}
					<div class="flex items-baseline justify-between gap-2 text-muted-foreground">
						<span class="shrink-0">{row.label}</span>
						<span
							class={cn(
								'min-w-0 text-right [overflow-wrap:anywhere]',
								row.mono ? 'font-mono' : 'tabular-nums',
								i === 0 && 'font-medium',
								statusTextPlain[row.tone ?? 'neutral']
							)}
						>
							{row.value}
						</span>
					</div>
				{/each}
			</div>
		{/if}
	</div>

	<div class="mt-3 flex items-center justify-between border-t border-border/60 pt-2 text-[11px] text-muted-foreground">
		<span>{active ? 'Hide details' : 'Inspect details'}</span>
		{#if active}
			<ChevronUpIcon class="size-3" />
		{:else}
			<ChevronDownIcon class="size-3" />
		{/if}
	</div>
</button>
