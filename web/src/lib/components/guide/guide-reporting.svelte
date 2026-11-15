<script lang="ts">
	import { resolve } from '$app/paths';
	import InlineCode from '$lib/components/inline-code.svelte';
	import type { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { plural } from '$lib/format';
	import { FORWARDED_NAMED, MAILBOX_AUTHORIZATION_LABEL, REPORTING_STATUS_LABEL } from '$lib/report-address';
	import { statusDot, TONE_RANK } from '$lib/tokens';
	import type { ReportingDomain } from '$lib/types/ReportingDomain';
	import type { Status } from '$lib/types/Status';
	import { cn } from '$lib/utils.js';
	import DocsLink from './docs-link.svelte';
	import GuideBlock from './guide-block.svelte';

	/**
	 * The Guide's first two blocks, from the reporting setup the setup checks (`reporting`): the
	 * addresses reports reach Marcan at (every receiving account's, with the ones forwarded to it)
	 * and how many domains report there, then each receiving domain's permission as the server
	 * decided it. With no receiving account the first block says so and links to Accounts.
	 */
	let { reporting, importing = false }: { reporting: ReportingSetupState; importing?: boolean } = $props();

	/** How many domains not reporting are named before "and n more". */
	const NAMED_MAX = 5;

	const data = $derived(reporting.data);
	const pending = $derived(data === null && !reporting.source.error);
	const failed = $derived(data === null && !!reporting.source.error);
	const addresses = $derived(data?.addresses ?? []);
	const domains = $derived(data?.domains ?? []);
	const authorizations = $derived(data?.authorizations ?? []);
	const setupDmarc = `${resolve('/setup')}?step=dmarc`;
	const setupAddresses = `${resolve('/setup')}?step=addresses`;

	/** The worst of the rows' verdicts. */
	function worst(rows: readonly ReportingDomain[]): Status {
		return rows.reduce<Status>((w, r) => (TONE_RANK[r.status] > TONE_RANK[w] ? r.status : w), 'pass');
	}

	/** A status label ("Waiting for reports") inside a sentence: its first letter in lower case. */
	const midSentence = (label: string) => label.charAt(0).toLowerCase() + label.slice(1);

	/** Before the `i`th of `n` items in a sentence: nothing, a comma, or "and". */
	const separator = (i: number, n: number) => (i === 0 ? '' : i === n - 1 ? ' and ' : ', ');
</script>

{#snippet dot(tone: Status)}
	<span class={cn('mr-1.5 inline-block size-2 shrink-0 rounded-full align-middle', statusDot[tone])} aria-hidden="true"
	></span>
{/snippet}

{#snippet statusSkeleton()}
	<Skeleton class="h-4 w-56" />
{/snippet}

{#snippet checkFailed()}
	<span class="text-muted-foreground">{@render dot('neutral')}Check failed.</span>
	<button type="button" class="text-foreground underline underline-offset-2" onclick={() => reporting.recheck()}>
		Re-check
	</button>
{/snippet}

{#snippet fixLink(href: string, text: string)}
	<a {href} class="text-foreground underline underline-offset-2">{text}</a>
{/snippet}

<!-- An account's forwarded addresses after its own: named up to FORWARDED_NAMED, else counted. -->
{#snippet forwarded(from: string[])}{#if from.length > FORWARDED_NAMED}{' '}(and {plural(
			from.length,
			'address',
			'addresses'
		)} forwarded to it){:else if from.length > 0}{' '}(forwarded from {#each from as f, j (f)}{separator(
				j,
				from.length
			)}<InlineCode>{f}</InlineCode>{/each}){/if}{/snippet}

<!-- 1. The rua= addresses. -->
{#snippet reachLinks()}<DocsLink path="accounts#the-report-address" label="Accounts & reporting setup" />{/snippet}
{#snippet reachBody()}
	{#if pending}
		<Skeleton class="h-4 w-full" />
	{:else if addresses.length > 0 && data}
		Reports reach Marcan at
		{#each addresses as a, i (a.account_id)}{separator(i, addresses.length)}<InlineCode>{a.address}</InlineCode
			>{@render forwarded(a.forwarded_from)}{/each}. Every domain you monitor must name {addresses.length > 1
			? 'one of them'
			: 'it'} in the <InlineCode>rua=</InlineCode> tag of its DMARC record{#if addresses.length > 1}; new records name <InlineCode
				>{data.default_address}</InlineCode
			>{/if}.
	{:else if failed}
		Reports reach Marcan at the addresses of the accounts that receive them, named in the <InlineCode>rua=</InlineCode> tag
		of each domain's DMARC record.
	{:else}
		No account receives reports yet: <SettingsLink to="accounts">add one under Settings → Accounts</SettingsLink>. Every
		domain you monitor must then name its address in the <InlineCode>rua=</InlineCode> tag of its DMARC record.
	{/if}
{/snippet}
{#snippet reachStatus()}
	{#if pending}
		{@render statusSkeleton()}
	{:else if failed}
		{@render checkFailed()}
	{:else if domains.length === 0}
		<span class="text-muted-foreground">{@render dot('neutral')}No domains yet.</span>
	{:else}
		{@const reportingCount = domains.filter((d) => d.state === 'reporting').length}
		{@const missing = domains.filter((d) => d.state !== 'reporting')}
		{@render dot(worst(domains))}{reportingCount} of {plural(domains.length, 'domain')} report here{importing
			? ' so far; the import is still running'
			: ''}.
		{#if missing.length > 0}
			{@const more = missing.length - NAMED_MAX}
			<span class="text-muted-foreground">
				Not yet:
				{#each missing.slice(0, NAMED_MAX) as d, i (d.id)}{#if i > 0},{/if}
					<a href={resolve(`/domains/${d.id}`)} class="text-foreground underline underline-offset-2">{d.domain}</a>
					({midSentence(REPORTING_STATUS_LABEL[d.state])}){/each}{more > 0 ? `, and ${more} more` : ''}.
			</span>
			{#if missing.some((d) => !d.set_up)}
				{@render fixLink(setupDmarc, 'Fix the records in the setup')}
			{/if}
		{/if}
	{/if}
{/snippet}
<GuideBlock
	id="reach"
	title="How reports reach Marcan"
	links={reachLinks}
	body={reachBody}
	status={pending || failed || addresses.length > 0 ? reachStatus : undefined}
/>

<!-- 2. Permission to receive reports (RFC 9990 §4). -->
{#snippet authLinks()}<DocsLink path="accounts#authorization" label="Authorization" />{/snippet}
{#snippet authBody()}
	A domain whose reports go to an address on another organizational domain needs that domain's permission to receive
	them: a <InlineCode>{'<domain>._report._dmarc.<receiving domain>'}</InlineCode> record with
	<InlineCode>v=DMARC1</InlineCode>, or one <InlineCode>{'*._report._dmarc.<receiving domain>'}</InlineCode> record for every
	domain.
{/snippet}
{#snippet authStatus()}
	{#if pending}
		{@render statusSkeleton()}
	{:else if failed}
		{@render checkFailed()}
	{:else}
		<ul class="space-y-0.5">
			{#each authorizations as a (a.domain)}
				{@const label = MAILBOX_AUTHORIZATION_LABEL[a.state]}
				{@const refused = domains.filter((d) => d.missing_authorizations.includes(a.domain)).length}
				<li>
					{#if label === null}
						<span class="text-muted-foreground"
							>{@render dot('neutral')}<InlineCode>{a.domain}</InlineCode>: Not needed.</span
						>
					{:else}
						{@render dot(a.status)}<InlineCode>{a.domain}</InlineCode>: {label}{refused > 0
							? `, ${plural(refused, 'domain')} without permission`
							: ''}.
						{#if a.state === 'no_wildcard' || a.state === 'check_failed'}
							{@render fixLink(setupAddresses, 'Publish it in the setup')}
						{/if}
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
{/snippet}
<GuideBlock
	id="permission"
	title="Permission to receive reports"
	links={authLinks}
	body={authBody}
	status={addresses.length > 0 || pending || failed ? authStatus : undefined}
/>
