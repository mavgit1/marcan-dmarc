<script lang="ts">
	import type { PolicySource } from '$lib/types/PolicySource';
	import { POLICY_SOURCE_TEXT } from '$lib/wording/policy-source';
	import type { Enforcement } from '$lib/types/Enforcement';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { enforcementLabel } from '$lib/health';

	/**
	 * The enforcement axis: what receivers are asked to do with failing mail (the server's verdict).
	 * A domain's policy is its DMARC record in DNS (`policy_source` `dns`); without one, a state in
	 * place of the policy (`POLICY_SOURCE_TEXT`). A report's own policy is shown only with that report.
	 */
	let {
		enforcement,
		policy,
		source,
		class: className
	}: {
		enforcement: Enforcement;
		/** The published value, shown as is when it is not none / quarantine / reject. */
		policy: string | null | undefined;
		/** A domain's `policy_source`; omitted where the policy is a record or report of its own. */
		source?: PolicySource;
		class?: string;
	} = $props();

	const text = $derived(source ? POLICY_SOURCE_TEXT[source] : null);
</script>

<span class="inline-flex" title={text?.title}>
	<StatusBadge tone={enforcement.status} class={className}>
		{text?.label ?? enforcementLabel(enforcement, policy)}
	</StatusBadge>
</span>
