<script lang="ts">
	import type { SenderIdentity } from '$lib/types/SenderIdentity';
	import { displayName, identityEvidence, isUnconfirmedOperatorName, UNCONFIRMED_PTR_HINT } from '$lib/identity';
	import { cn } from '$lib/utils.js';

	/**
	 * Neutral sender caption. Marks unconfirmed operator names only - never a behaviour verdict.
	 */
	let {
		identity,
		class: className
	}: {
		identity: SenderIdentity | null | undefined;
		class?: string;
	} = $props();

	const name = $derived(displayName(identity));
	const unconfirmed = $derived(isUnconfirmedOperatorName(identity));
	const evidence = $derived(identityEvidence(identity) ?? undefined);
</script>

{#if unconfirmed}
	<span
		class={cn(
			'text-muted-foreground underline decoration-dashed decoration-muted-foreground/60 underline-offset-2',
			className
		)}
		title={UNCONFIRMED_PTR_HINT}
	>
		{name}
	</span>
{:else}
	<span class={className} title={evidence}>{name}</span>
{/if}
