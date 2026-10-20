<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import { resolve } from '$app/paths';
	import type { ReportingMode } from '$lib/types/ReportingMode';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { emailDomain, MAILBOX_AUTHORIZATION_LABEL } from '$lib/report-address';
	import type { Snippet } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import ReportingDomain from './reporting-domain.svelte';
	import type { ReportingSetupState } from './reporting-setup-state.svelte';

	/**
	 * How to get DMARC reports into Marcan. Direct mode: authorize the mailbox domain, then point
	 * each domain's rua at the report address. Forwarded mode: an address per domain that
	 * forwards here, the DMARC record naming it, and permission only when that address is on
	 * another domain. With `setup`, every domain's status as the server decided it; with no
	 * domains (or no report address yet) the same instructions render for a placeholder domain.
	 * Its parts are headed, never numbered: the setup's step list is the one numbering. The UI
	 * parses and edits no DMARC record text; the records to publish are the server's
	 * (`record_to_publish`).
	 */
	let {
		setup,
		reportAddress = '',
		reportingMode = 'direct',
		addressHint,
		exampleBelow = false,
		importing = false
	}: {
		/** The server's reporting setup; without it, the example only. */
		setup?: ReportingSetupState;
		/** The address and mode for the example, without `setup`. */
		reportAddress?: string;
		reportingMode?: ReportingMode;
		/** Shown in example mode when no report address is configured; defaults to a Settings link. */
		addressHint?: Snippet;
		/** In live mode, also offer the example (collapsed) for a domain not listed yet. */
		exampleBelow?: boolean;
		/** A mailbox pass runs: the list is not complete, so no "n of m already report". */
		importing?: boolean;
	} = $props();

	const data = $derived(setup?.data ?? null);
	/** The first answer is still out. */
	const pending = $derived(!!setup && data === null && !setup.source.error);
	const address = $derived((data ? data.report_address : reportAddress).trim());
	const mode = $derived(data?.reporting_mode ?? reportingMode);
	const direct = $derived(mode === 'direct');
	const domains = $derived(data?.domains ?? []);
	const live = $derived(domains.length > 0 && address !== '');
	const mailboxDomain = $derived(data?.mailbox_domain || emailDomain(address) || 'example.net');
	const shownAddress = $derived(address || `reports@${mailboxDomain}`);
	/** Placeholder monitored domain, kept apart from the mailbox domain. */
	const exampleDomain = $derived(mailboxDomain === 'example.com' ? 'example.org' : 'example.com');
	/** Another domain an existing rua address is on. */
	const providerHost = 'vendor.example';
	const exampleExisting = $derived(`v=DMARC1; p=quarantine; rua=mailto:dmarc@${providerHost}`);
	/** The rua address the example adds: this mailbox, or (forwarded) the domain's own address. */
	const exampleRua = $derived(direct ? shownAddress : `dmarc@${exampleDomain}`);
	/** Another domain a forwarding address could be on. */
	const forwardHost = 'msp.example';
	/** The domain named in the forwarded-mode text: the one listed, else a placeholder. */
	const sampleDomain = $derived(live ? (domains.length === 1 ? domains[0].domain : '<domain>') : exampleDomain);

	/** The mailbox domain's authorization, as the server decided it (direct mode). */
	const auth = $derived(data?.mailbox_authorization ?? null);
	const authLabel = $derived(auth ? MAILBOX_AUTHORIZATION_LABEL[auth.state] : null);
	const reportingCount = $derived(domains.filter((d) => d.state === 'reporting').length);
</script>

{#snippet heading(text: string, badge?: Snippet)}
	<h3 class="flex flex-wrap items-center gap-2 text-sm font-semibold">
		{text}
		{@render badge?.()}
	</h3>
{/snippet}

{#snippet authBadge()}
	{#if pending}
		<Skeleton class="h-4 w-20" />
	{:else if auth && authLabel}
		<StatusBadge tone={auth.status} class="text-xs">{authLabel}</StatusBadge>
	{/if}
{/snippet}

{#snippet recheck()}
	{#if setup && !pending}
		<Button variant="ghost" size="sm" class="gap-1.5" disabled={setup.source.loading} onclick={() => setup.recheck()}>
			<RefreshCwIcon class="size-3.5" />
			Re-check
		</Button>
	{/if}
{/snippet}

<div class="space-y-4 text-xs leading-relaxed">
	{#if !address}
		<p class="text-muted-foreground">
			{#if addressHint}
				{@render addressHint()}
			{:else}
				No report address yet: <InlineCode>{shownAddress}</InlineCode> is a placeholder. Set yours in
				<a href={resolve('/settings/mailbox')} class="text-foreground font-medium underline underline-offset-2"
					>Settings → Mailbox</a
				>.
			{/if}
		</p>
	{/if}

	{#if direct}
		<section class="space-y-2">
			{@render heading(`Authorize ${mailboxDomain}`, authBadge)}
			{#if auth?.state === 'not_needed'}
				<p class="text-muted-foreground">
					Not needed: your mailbox is on <InlineCode>{mailboxDomain}</InlineCode>, under the same organizational domain
					as the ones you monitor.
				</p>
			{:else if auth?.state === 'wildcard'}
				<p class="text-muted-foreground">
					<InlineCode>{`*._report._dmarc.${mailboxDomain}`}</InlineCode> is published: every domain may send reports here.
				</p>
			{:else if auth?.state === 'per_domain'}
				<div class="text-muted-foreground flex flex-wrap items-center justify-between gap-2">
					<p>
						Each domain has its own authorization record. A wildcard record would also cover domains you add later: <InlineCode
							>{`*._report._dmarc.${mailboxDomain}`}</InlineCode
						> with the value <InlineCode>{'v=DMARC1'}</InlineCode>.
					</p>
					{@render recheck()}
				</div>
			{:else}
				<p class="text-muted-foreground">
					Your mailbox is on <InlineCode>{mailboxDomain}</InlineCode>. Receivers only send another domain's reports
					there if
					<InlineCode>{mailboxDomain}</InlineCode> agrees. Recommended: this one wildcard record covers every domain, including
					ones you add later.
				</p>
				<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,8rem)]">
					<DnsRawRecord value={`*._report._dmarc.${mailboxDomain}`} label="Host (TXT)" />
					<DnsRawRecord value="v=DMARC1" label="Value" />
				</div>
				<div class="text-muted-foreground flex flex-wrap items-center justify-between gap-2">
					<p>
						Or one record per domain instead: <InlineCode>{`<domain>._report._dmarc.${mailboxDomain}`}</InlineCode>{live
							? ' (checked per domain below)'
							: ''}. Neither is needed for domains under the mailbox's own organizational domain.
					</p>
					{#if auth}
						{@render recheck()}
					{/if}
				</div>
			{/if}
		</section>
	{/if}

	{#if !direct}
		<section class="space-y-2">
			{@render heading('A report address per domain')}
			<p class="text-muted-foreground">
				For each domain, create an address such as <InlineCode>{`dmarc@${sampleDomain}`}</InlineCode> that forwards to
				<InlineCode>{shownAddress}</InlineCode>. A domain may also name <InlineCode>{shownAddress}</InlineCode> directly.
			</p>
		</section>
	{/if}

	<section class="space-y-2">
		{@render heading(live && domains.length === 1 ? 'DMARC record' : 'DMARC record per domain')}
		{#if pending}
			<Skeleton class="h-24 w-full" />
		{:else if setup?.source.error && !data}
			<ErrorState title="Reporting setup failed to load" error={setup.source.error} onRetry={() => setup.recheck()} />
		{:else if live && setup}
			{#if !importing && domains.length > 1 && reportingCount > 0}
				<p class="text-muted-foreground">
					{reportingCount} of {domains.length} already report to Marcan{reportingCount < domains.length
						? '; the others are listed first'
						: ''}.
				</p>
			{/if}
			<ul class="divide-border divide-y rounded-lg border">
				{#each domains as row (row.id)}
					<ReportingDomain {row} {setup} {mode} {mailboxDomain} />
				{/each}
			</ul>
			{#if exampleBelow}
				<details class="group text-muted-foreground">
					<summary class="hover:text-foreground cursor-pointer list-none text-xs select-none">
						<span class="group-open:hidden">▸</span><span class="hidden group-open:inline">▾</span>
						Setting up a domain that isn't listed? Same steps, shown for an example
					</summary>
					<div class="mt-2">{@render example()}</div>
				</details>
			{/if}
		{:else}
			{@render example()}
		{/if}
	</section>

	{#if !direct}
		<section class="space-y-2">
			{@render heading('Permission, only for an address on another domain')}
			<p class="text-muted-foreground">
				An address on the domain itself (like <InlineCode>{`dmarc@${sampleDomain}`}</InlineCode>) needs nothing more. If
				it is on another domain, e.g. <InlineCode>{`reports@${forwardHost}`}</InlineCode>, that domain has to accept the
				reports{live ? ' (flagged per domain above when missing)' : ''}:
			</p>
			<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,8rem)]">
				<DnsRawRecord value={`${sampleDomain}._report._dmarc.${forwardHost}`} label="Host (TXT)" />
				<DnsRawRecord value="v=DMARC1" label="Value" />
			</div>
		</section>
	{/if}
</div>

{#snippet example()}
	<div class="space-y-3 rounded-lg border p-3">
		<div class="flex items-center gap-2">
			<span class="font-mono text-sm font-medium">{exampleDomain}</span>
			<StatusBadge tone="neutral" class="text-xs">Example</StatusBadge>
		</div>
		<p class="text-muted-foreground">
			No DMARC record yet? Publish this TXT record; <InlineCode>{'p=none'}</InlineCode> only monitors and does not affect
			delivery.
		</p>
		<div class="grid gap-3 sm:grid-cols-[minmax(0,12rem)_minmax(0,1fr)]">
			<DnsRawRecord value={`_dmarc.${exampleDomain}`} label="Host (TXT)" />
			<DnsRawRecord value={`v=DMARC1; p=none; rua=mailto:${exampleRua}`} label="Value" />
		</div>
		<p class="text-muted-foreground">
			Already have one, e.g. <InlineCode>{exampleExisting}</InlineCode>? Keep it and its policy; in
			<InlineCode>{'rua='}</InlineCode> either replace the address with <InlineCode>{exampleRua}</InlineCode> or add yours
			after a comma.
		</p>
	</div>
{/snippet}
