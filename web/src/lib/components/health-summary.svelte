<script lang="ts">
	import type { MailCounts } from '#lib/types/MailCounts.ts';
	import type { MailVerdicts } from '#lib/types/MailVerdicts.ts';
	import HealthBadge from '#lib/components/health-badge.svelte';
	import OutcomeLegend from '#lib/components/outcome-legend.svelte';
	import type { OutcomeFigures } from '#lib/components/outcome-legend.ts';
	import Skeleton from '#lib/components/skeleton.svelte';
	import { formatInt } from '#lib/format.ts';
	import { formatRate } from '#lib/health.ts';
	import { statusFigure } from '#lib/tokens.ts';
	import { cn } from '#lib/utils.ts';

	/**
	 * A report's posture strip: the server's health verdict for the report as its badge with the
	 * delivered compliance it rests on beside it, then the legend with the four outcomes, a report
	 * having no chart to carry it. The dashboard has no health card: a scope-wide badge said nothing the
	 * Delivered compliance card and the attention list do not.
	 */
	let {
		mail,
		outcomes,
		loading = false,
		class: className
	}: {
		/** The report's figures plus the server's verdicts. */
		mail: (MailCounts & MailVerdicts) | null;
		/** The legend with the counts, under the badge. */
		outcomes: OutcomeFigures;
		loading?: boolean;
		class?: string;
	} = $props();
</script>

<section
	class={cn('bg-card ring-foreground/10 flex flex-col gap-3 rounded-xl p-5 ring-1', className)}
	aria-label="Authentication health"
>
	{#if loading || !mail}
		<Skeleton class="h-6 w-24" />
	{:else}
		<div class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
			<HealthBadge health={mail.health} />
			<span class="text-sm">
				<span class="text-muted-foreground">Delivered compliance</span>
				<span class={cn('font-semibold tabular-nums', statusFigure[mail.delivered_compliance.status])}
					>{formatRate(mail.delivered_compliance)}</span
				>
			</span>
			<span class="text-muted-foreground text-xs">
				{formatInt(mail.compliant_messages)} of {formatInt(mail.delivered_messages)} delivered messages passed DMARC
			</span>
		</div>
	{/if}
	<OutcomeLegend class="border-t pt-3" {...outcomes} mail={loading ? null : outcomes.mail} />
</section>
