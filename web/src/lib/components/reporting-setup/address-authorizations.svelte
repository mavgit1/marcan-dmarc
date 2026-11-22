<script lang="ts">
	import InlineCode from '#lib/components/inline-code.svelte';
	import DnsRawRecord from '#lib/components/dns-raw-record.svelte';
	import StatusBadge from '#lib/components/status-badge.svelte';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { MAILBOX_AUTHORIZATION_LABEL } from '#lib/report-address.ts';
	import { joinAnd } from '#lib/text.ts';
	import type { AddressAuthorization } from '#lib/types/AddressAuthorization.ts';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import type { ReportingSetupState } from './reporting-setup-state.svelte';

	/**
	 * The authorization each report address domain must publish (RFC 9990 §4), as the server
	 * decided it (`authorizations`): one part per address domain that some monitored domain
	 * outside its organizational domain sends reports to, with the real records to publish. An
	 * address domain that needs nothing is left out; with none left, one line says so.
	 */
	let { setup }: { setup: ReportingSetupState } = $props();

	const authorizations = $derived(setup.data?.authorizations ?? []);
	const shown = $derived(authorizations.filter((a) => a.state !== 'not_needed'));
	const anyDomain = $derived((setup.data?.domains.length ?? 0) > 0);
</script>

{#snippet recheck()}
	<Button variant="ghost" size="sm" class="gap-1.5" disabled={setup.source.loading} onclick={() => setup.recheck()}>
		<RefreshCwIcon class="size-3.5" />
		Re-check
	</Button>
{/snippet}

{#snippet part(a: AddressAuthorization)}
	{@const label = MAILBOX_AUTHORIZATION_LABEL[a.state]}
	{@const wildcard = `*._report._dmarc.${a.domain}`}
	<section class="space-y-2" aria-label={`Authorization of ${a.domain}`}>
		<h3 class="flex flex-wrap items-center gap-2 text-sm font-semibold">
			Authorize {a.domain}
			{#if label}
				<StatusBadge tone={a.status} class="text-xs">{label}</StatusBadge>
			{/if}
		</h3>
		{#if a.state === 'wildcard'}
			<p class="text-muted-foreground">
				<InlineCode>{wildcard}</InlineCode> is published: every domain may send reports to {joinAnd(a.addresses)}.
			</p>
		{:else if a.state === 'per_domain'}
			<div class="text-muted-foreground flex flex-wrap items-center justify-between gap-2">
				<p>
					Each domain has its own authorization record. A wildcard record would also cover domains you add later: <InlineCode
						>{wildcard}</InlineCode
					> with the value <InlineCode>{'v=DMARC1'}</InlineCode>.
				</p>
				{@render recheck()}
			</div>
		{:else}
			<p class="text-muted-foreground">
				Receivers only send another domain's reports to {joinAnd(a.addresses)} if <InlineCode>{a.domain}</InlineCode> agrees.
				Recommended: this one wildcard record on <InlineCode>{a.domain}</InlineCode> covers every domain, including ones you
				add later.
			</p>
			<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,8rem)]">
				<DnsRawRecord value={wildcard} label="Host (TXT)" />
				<DnsRawRecord value="v=DMARC1" label="Value" />
			</div>
			<div class="text-muted-foreground flex flex-wrap items-center justify-between gap-2">
				<p>
					Or one record per domain instead: <InlineCode>{`<domain>._report._dmarc.${a.domain}`}</InlineCode>{anyDomain
						? ' (checked per domain in the DMARC records)'
						: ''}. Neither is needed for domains under <InlineCode>{a.org_domain}</InlineCode>.
				</p>
				{@render recheck()}
			</div>
		{/if}
	</section>
{/snippet}

<div class="space-y-4 text-xs leading-relaxed">
	{#each shown as a (a.domain)}
		{@render part(a)}
	{:else}
		<p class="text-muted-foreground">
			{#if authorizations.length === 1}
				Not needed: {joinAnd(authorizations[0].addresses)} is on <InlineCode>{authorizations[0].domain}</InlineCode>,
				under the same organizational domain as the domains you monitor.
			{:else}
				Not needed: each domain sends its reports to an address under its own organizational domain.
			{/if}
		</p>
	{/each}
</div>
