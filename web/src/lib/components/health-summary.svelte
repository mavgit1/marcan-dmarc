<script lang="ts">
	import type { MailCounts } from '$lib/types/MailCounts';
	import type { MailVerdicts } from '$lib/types/MailVerdicts';
	import HealthBadge from '$lib/components/health-badge.svelte';
	import OutcomeLegend from '$lib/components/outcome-legend.svelte';
	import type { OutcomeFigures } from '$lib/components/outcome-legend';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { cn } from '$lib/utils.js';

	/**
	 * A report's posture strip: the server's health verdict for the report as its badge (the
	 * rate is the stat card's), then the legend with the four outcomes, a report having no
	 * chart to carry it. The dashboard has no health card: a scope-wide badge said nothing the
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
		<div class="flex items-center">
			<HealthBadge health={mail.health} />
		</div>
	{/if}
	<OutcomeLegend class="border-t pt-3" {...outcomes} mail={loading ? null : outcomes.mail} />
</section>
