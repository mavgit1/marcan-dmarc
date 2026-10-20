<script lang="ts">
	import { formatInt } from '$lib/format';
	import { outcomeCounts, type MailFigures, type OutcomeCounts } from '$lib/health';
	import { cn } from '$lib/utils.js';
	import { OUTCOMES } from '$lib/wording/outcome';

	/**
	 * A proportion bar of the four outcomes, for a list row without room for numbers (the
	 * organizations list): the reader scans the rows for the one that is not green. Each
	 * segment grows with its count; a non-zero one is at least 3 px wide, the room taken from
	 * the larger ones, so a small share is still seen. The counts are in each segment's title
	 * and the bar's label. Anywhere else, `OutcomeLegend`.
	 */
	let { mail, class: className }: { mail: MailFigures; class?: string } = $props();

	const values: OutcomeCounts = $derived(outcomeCounts(mail));
	const totalMessages = $derived(mail.total_messages);
</script>

<div
	class={cn('bg-muted flex h-2 overflow-hidden rounded-full', className)}
	role="img"
	aria-label={OUTCOMES.map((o) => `${formatInt(values[o.key])} ${o.label.toLowerCase()}`).join(', ')}
>
	{#if totalMessages > 0}
		{#each OUTCOMES as o (o.key)}
			{#if values[o.key] > 0}
				<div
					class={cn('h-full min-w-[3px] basis-0', o.color)}
					style:flex-grow={values[o.key]}
					title={`${o.label}: ${formatInt(values[o.key])}`}
				></div>
			{/if}
		{/each}
	{/if}
</div>
