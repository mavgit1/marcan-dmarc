<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import { resolve } from '$app/paths';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import type { Snippet } from 'svelte';
	import ReportingDomain from './reporting-domain.svelte';
	import type { ReportingSetupState } from './reporting-setup-state.svelte';

	/**
	 * Each monitored domain's DMARC record as the server decided it (`setup`): every domain's
	 * status, and the records to publish, which name the default report address (the first
	 * receiving account's). With no domains the same instructions render for `<domain>`. With no
	 * receiving account it says so and shows no record: every address in a record to publish is
	 * the installation's own. The address domains' authorization is the Report addresses step's
	 * (`address-authorizations.svelte`). Its parts are headed, never numbered: the setup's step
	 * list is the one numbering. The UI parses and edits no DMARC record text; the records to
	 * publish are the server's (`record_to_publish`).
	 */
	let {
		setup,
		addressHint,
		exampleBelow = false,
		importing = false
	}: {
		/** The server's reporting setup. */
		setup: ReportingSetupState;
		/** Shown in place of the records when no account receives reports; defaults to a Settings link. */
		addressHint?: Snippet;
		/** Also offer the example (collapsed) for a domain not listed yet. */
		exampleBelow?: boolean;
		/** A mailbox pass runs: the list is not complete, so no "n of m already report". */
		importing?: boolean;
	} = $props();

	const data = $derived(setup.data);
	/** The first answer is still out. */
	const pending = $derived(data === null && !setup.source.error);
	const address = $derived(data?.default_address ?? '');
	const domains = $derived(data?.domains ?? []);
	const live = $derived(domains.length > 0 && address !== '');
	/** A domain not listed yet, as a format hint. */
	const exampleDomain = '<domain>';
	const reportingCount = $derived(domains.filter((d) => d.state === 'reporting').length);
</script>

<div class="space-y-4 text-xs leading-relaxed">
	{#if pending}
		<Skeleton class="h-24 w-full" />
	{:else if setup.source.error && !data}
		<ErrorState title="Reporting setup failed to load" error={setup.source.error} onRetry={() => setup.recheck()} />
	{:else if !address}
		<p class="text-muted-foreground">
			{#if addressHint}
				{@render addressHint()}
			{:else}
				No account receives reports yet, so there is no record to publish. Add one in
				<a href={resolve('/settings/accounts')} class="text-foreground font-medium underline underline-offset-2"
					>Settings → Accounts</a
				>.
			{/if}
		</p>
	{:else}
		<section class="space-y-2">
			<h3 class="text-sm font-semibold">{live && domains.length === 1 ? 'DMARC record' : 'DMARC record per domain'}</h3>
			{#if live}
				{#if !importing && domains.length > 1 && reportingCount > 0}
					<p class="text-muted-foreground">
						{reportingCount} of {domains.length} already report to Marcan{reportingCount < domains.length
							? '; the others are listed first'
							: ''}.
					</p>
				{/if}
				<ul class="divide-border divide-y rounded-lg border">
					{#each domains as row (row.id)}
						<ReportingDomain {row} {setup} defaultAddress={address} />
					{/each}
				</ul>
				{#if exampleBelow}
					<details class="group text-muted-foreground">
						<summary class="hover:text-foreground cursor-pointer list-none text-xs select-none">
							<span class="group-open:hidden">▸</span><span class="hidden group-open:inline">▾</span>
							Setting up a domain that isn't listed? The same steps for any domain
						</summary>
						<div class="mt-2">{@render example()}</div>
					</details>
				{/if}
			{:else}
				{@render example()}
			{/if}
		</section>
	{/if}
</div>

{#snippet example()}
	<div class="space-y-3 rounded-lg border p-3">
		<div class="flex items-center gap-2">
			<span class="font-mono text-sm font-medium">{exampleDomain}</span>
			<StatusBadge tone="neutral" class="text-xs">Any domain</StatusBadge>
		</div>
		<p class="text-muted-foreground">
			No DMARC record yet? Publish this TXT record; <InlineCode>{'p=none'}</InlineCode> only monitors and does not affect
			delivery.
		</p>
		<div class="grid gap-3 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
			<DnsRawRecord value={`_dmarc.${exampleDomain}`} label="Host (TXT)" />
			<DnsRawRecord value={`v=DMARC1; p=none; rua=mailto:${address}`} label="Value" />
		</div>
		<p class="text-muted-foreground">
			Already have one? Keep it and its policy; in
			<InlineCode>{'rua='}</InlineCode> either replace the address with <InlineCode>{address}</InlineCode> or add it after
			a comma.
		</p>
	</div>
{/snippet}
