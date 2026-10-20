<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import type { ReportingMode } from '$lib/types/ReportingMode';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { PLACEHOLDER_ADDRESS, REPORTING_MODES } from '$lib/reporting-modes';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';

	/**
	 * The two reporting modes as selectable cards (radio group). The setup and Settings save
	 * the choice.
	 */
	let {
		value = $bindable(),
		name,
		address,
		current = null,
		legend
	}: {
		value: ReportingMode;
		/** Radio group name, unique per page. */
		name: string;
		/** Mailbox address for the examples; a placeholder when empty. */
		address: string;
		/** Marks the saved mode as "Your current setup". */
		current?: ReportingMode | null;
		legend?: string;
	} = $props();

	const shown = $derived(address.trim() || PLACEHOLDER_ADDRESS);
</script>

<!-- Side by side only when each card gets enough width (not in a half-width Settings card). -->
<fieldset class="@container">
	{#if legend}
		<legend class="text-muted-foreground mb-1 text-xs font-medium">{legend}</legend>
	{/if}
	<div class="grid gap-3 @xl:grid-cols-2">
		{#each REPORTING_MODES as mode (mode.value)}
			{@const example = mode.example(shown)}
			<label
				class="has-[:checked]:border-primary has-[:checked]:ring-primary/30 has-[:focus-visible]:ring-ring/50 bg-card flex cursor-pointer flex-col gap-2 rounded-lg border p-3 text-xs leading-relaxed has-[:checked]:ring-1 has-[:focus-visible]:ring-[3px]"
			>
				<span class="flex flex-wrap items-center gap-2">
					<input type="radio" {name} value={mode.value} bind:group={value} class="accent-primary" />
					<span class="text-sm font-semibold">{mode.title}</span>
					{#if mode.recommended}
						<StatusBadge tone="neutral" class="text-[11px]">Recommended</StatusBadge>
					{/if}
					{#if current === mode.value}
						<StatusBadge tone="pass" class="text-[11px]">Your current setup</StatusBadge>
					{/if}
				</span>
				<span class="text-foreground font-medium">{mode.fitsIf}</span>
				<span class="text-muted-foreground">{mode.explain(shown)}</span>
				<span class="text-muted-foreground mt-auto flex flex-wrap items-center gap-x-1.5 gap-y-1 pt-1">
					<span class="text-[11px]">Example</span>
					<InlineCode>{example.record}</InlineCode>
					{#if example.forwardsTo}
						<span class="flex items-center gap-1.5">
							<ArrowRightIcon class="size-3 shrink-0" aria-label="forwards to" />
							<InlineCode>{example.forwardsTo}</InlineCode>
						</span>
					{:else if example.note}
						<span>{example.note}</span>
					{/if}
				</span>
			</label>
		{/each}
	</div>
</fieldset>
