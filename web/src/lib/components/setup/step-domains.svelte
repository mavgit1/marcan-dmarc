<script lang="ts">
	import { ignoreDomain } from '$lib/api';
	import { apiErrorMessage } from '$lib/http';
	import { Action } from '$lib/components/action.svelte';
	import type { ScopeDomain } from '$lib/types/ScopeDomain';
	import AddDomains from '$lib/components/add-domains.svelte';
	import BusyButton from '$lib/components/busy-button.svelte';
	import { plural } from '$lib/format';
	import XIcon from '@lucide/svelte/icons/x';
	import { importing, type SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The monitored domains: the ones the import found (the list follows it) and any added here.
	 * Optional, since domains also appear with their first report.
	 */
	let {
		data,
		onChanged,
		onBack,
		onNext
	}: {
		data: SetupData;
		/** A domain was added or ignored; reload the list. */
		onChanged: () => void;
		onBack: () => void;
		onNext: () => void;
	} = $props();

	const domains = $derived(data.domains);
	const running = $derived(importing(data));

	/** Which chip is being ignored (one at a time). */
	let removing = $state<number | null>(null);
	const remove = new Action();

	/**
	 * Only a domain without reports (e.g. a typo just added) is ignored here, without
	 * confirmation: nothing is lost. One with reports is ignored on its own page.
	 */
	function onRemove(domain: ScopeDomain) {
		if (remove.busy) return;
		removing = domain.id;
		void remove.run(async () => {
			try {
				await ignoreDomain(domain.id);
				onChanged();
			} catch (err) {
				throw new Error(`${domain.domain}: ${apiErrorMessage(err)}`, { cause: err });
			} finally {
				removing = null;
			}
		});
	}
</script>

<div class="space-y-4">
	<p class="text-muted-foreground text-xs leading-relaxed">
		Optional: domains also appear with their first report. Add them now for DNS results right away. They go to
		<span class="text-foreground font-medium">{data.organization.name}</span>.
	</p>
	<p class="text-xs font-medium" role="status" aria-live="polite">
		{#if running}
			Still importing: {domains.length > 0 ? `${plural(domains.length, 'domain')} so far, ` : ''}the list grows as
			reports arrive.
		{:else if domains.length > 0}
			{plural(domains.length, 'domain')}.
		{:else}
			No domains yet.
		{/if}
	</p>
	{#if domains.length > 0}
		<ul class="flex flex-wrap gap-1.5" aria-label="Domains">
			{#each domains as domain (domain.id)}
				<li class="bg-muted/50 flex items-center gap-0.5 rounded-md border py-0.5 pr-0.5 pl-2">
					<span class="font-mono text-xs">{domain.domain}</span>
					{#if !domain.has_reports}
						<BusyButton
							variant="ghost"
							size="icon-xs"
							class="text-muted-foreground hover:text-foreground"
							busy={removing === domain.id}
							busyLabel=""
							disabled={remove.busy}
							aria-label={`Ignore ${domain.domain}`}
							title="Ignore: not monitored, its reports are dropped"
							onclick={() => onRemove(domain)}
						>
							<XIcon />
						</BusyButton>
					{:else}
						<span class="w-1"></span>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
	{#if remove.feedback && !remove.feedback.ok}
		<p class="text-status-fail-fg text-xs" role="alert">{remove.feedback.message}</p>
	{/if}
	<AddDomains organizationId={data.organization.id} onAdded={onChanged} />
	<StepNav {onBack} onContinue={onNext} />
</div>
