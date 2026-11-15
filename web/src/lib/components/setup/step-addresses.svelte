<script lang="ts">
	import ErrorState from '$lib/components/error-state.svelte';
	import InlineCode from '$lib/components/inline-code.svelte';
	import AddressAuthorizations from '$lib/components/reporting-setup/address-authorizations.svelte';
	import type { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StepNav from './step-nav.svelte';

	/**
	 * Where reports reach Marcan, read from the accounts as the server lists them: each receiving
	 * account's address and the one address forwarded to it, the address new DMARC records name
	 * (the first receiving account's), then the authorization records each address domain must
	 * publish, with the real addresses. Nothing to type here: an address is changed on its
	 * account. Continue is always open: DNS changes take time to show.
	 */
	let {
		reporting,
		onBack,
		onNext
	}: {
		reporting: ReportingSetupState;
		onBack: () => void;
		onNext: () => void;
	} = $props();

	const data = $derived(reporting.data);
	const pending = $derived(data === null && !reporting.source.error);
	const addresses = $derived(data?.addresses ?? []);
</script>

<div class="space-y-4 text-xs leading-relaxed">
	<p class="text-muted-foreground">
		Receivers send a domain's DMARC reports to the addresses in its DMARC record. These reach Marcan:
	</p>
	{#if pending}
		<Skeleton class="h-16 w-full" />
	{:else if reporting.source.error && !data}
		<ErrorState
			title="Reporting setup failed to load"
			error={reporting.source.error}
			onRetry={() => reporting.recheck()}
		/>
	{:else if addresses.length === 0}
		<p class="text-muted-foreground">No account receives reports yet: add one in the Accounts step.</p>
	{:else if data}
		<ul class="divide-border divide-y rounded-lg border" aria-label="Report addresses">
			{#each addresses as a (a.account_id)}
				<li class="space-y-0.5 px-3 py-2">
					<p class="text-sm font-medium break-words">{a.name}</p>
					<p class="text-muted-foreground break-words">
						Reports sent to <span class="text-foreground">{a.address}</span> land here
					</p>
					{#if a.forwarded_from.length > 0}
						<p class="text-muted-foreground break-words">
							Also reports forwarded from {#each a.forwarded_from as f, j (f)}{j === 0
									? ''
									: j === a.forwarded_from.length - 1
										? ' and '
										: ', '}<span class="text-foreground">{f}</span>{/each}
						</p>
					{/if}
				</li>
			{/each}
		</ul>
		<p class="text-muted-foreground">
			New DMARC records name <InlineCode>{data.default_address}</InlineCode>{addresses.length > 1
				? ', the first account above'
				: ''}. An address is changed on its account.
		</p>
		<AddressAuthorizations setup={reporting} />
	{/if}
	<StepNav {onBack} onContinue={onNext} />
</div>
