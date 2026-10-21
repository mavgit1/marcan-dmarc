<script lang="ts">
	import { resolve } from '$app/paths';
	import { getGuide } from '$lib/api';
	import type { ReportingDomain } from '$lib/types/ReportingDomain';
	import type { SettingsResponse } from '$lib/types/SettingsResponse';
	import type { Status } from '$lib/types/Status';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import InlineCode from '$lib/components/inline-code.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { formatDay, plural } from '$lib/format';
	import { DOCS_URL } from '$lib/nav';
	import { pipelineStatus } from '$lib/pipeline';
	import { MAILBOX_AUTHORIZATION_LABEL, REPORTING_STATUS_LABEL } from '$lib/report-address';
	import { meta } from '$lib/state/meta.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { statusDot, TONE_RANK } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import type { Snippet } from 'svelte';

	/**
	 * Tools → Guide: how Marcan works, block by block, with this installation's values and where
	 * each part stands, read from what the server already answers (the reporting setup the setup's
	 * DMARC step checks, the settings, the health, `/api/meta`) plus the policy counts
	 * (`/api/guide`). Read-only; the setup and the docs go deeper.
	 */

	/** How many domains not reporting are named before "and n more". */
	const NAMED_MAX = 5;

	const loaded = resource({
		key: () => 'guide',
		load: () => Promise.all([settings.load(), meta.load().catch(() => null)]).then(([s]) => s)
	});
	const guide = resource({ key: () => 'guide-policies', load: getGuide, reloadOn: () => systemHealth.dnsTick });
	/** The same check as the setup's DMARC step (the domains' records and the mailbox domain's permission). */
	const reporting = new ReportingSetupState({
		enabled: () => !!settings.current?.reporting.report_address,
		reloadOn: () => [settings.current?.reporting.report_address, settings.current?.reporting.reporting_mode]
	});

	const health = $derived(systemHealth.current);
	const pipeline = $derived(health ? pipelineStatus(health) : null);
	const importing = $derived(!!health?.imap_progress);
	const rules = $derived(meta.current);
	const setupDmarc = `${resolve('/setup')}?step=dmarc`;

	/** The worst of the rows' verdicts. */
	function worst(rows: readonly ReportingDomain[]): Status {
		return rows.reduce<Status>((w, r) => (TONE_RANK[r.status] > TONE_RANK[w] ? r.status : w), 'pass');
	}
</script>

{#snippet docs(path: string, label = 'Docs')}
	<a
		href="{DOCS_URL}{path}"
		target="_blank"
		rel="noopener"
		class="text-muted-foreground hover:text-foreground inline-flex items-center gap-1 text-xs underline-offset-2 hover:underline"
		>{label}<ExternalLinkIcon class="size-3" /></a
	>
{/snippet}

{#snippet block(id: string, title: string, links: Snippet, body: Snippet, status?: Snippet)}
	<section aria-labelledby="guide-{id}" class="space-y-1.5 py-4 first:pt-0">
		<div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
			<h2 id="guide-{id}" class="text-sm font-medium">{title}</h2>
			<div class="flex items-center gap-3">{@render links()}</div>
		</div>
		<p class="text-muted-foreground leading-relaxed">{@render body()}</p>
		{#if status}
			<div class="text-xs leading-relaxed">{@render status()}</div>
		{/if}
	</section>
{/snippet}

{#snippet dot(tone: Status)}
	<span class={cn('mr-1.5 inline-block size-2 shrink-0 rounded-full align-middle', statusDot[tone])} aria-hidden="true"
	></span>
{/snippet}

{#snippet fixLink(text: string)}
	<a href={setupDmarc} class="text-foreground underline underline-offset-2">{text}</a>
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

<div class="mx-auto max-w-3xl text-sm">
	<PageHeader
		title="Guide"
		description="How Marcan works, with this installation's values and where each part stands."
	/>
	<Loaded source={loaded} errorTitle="Guide failed to load" data={body} loading={skeleton} />
</div>

{#snippet skeleton()}
	<div class="space-y-6">
		{#each [0, 1, 2, 3, 4] as i (i)}
			<div class="space-y-2">
				<Skeleton class="h-4 w-48" />
				<Skeleton class="h-10 w-full" />
			</div>
		{/each}
	</div>
{/snippet}

{#snippet body(s: SettingsResponse)}
	{@const address = s.reporting.report_address}
	{@const direct = s.reporting.reporting_mode === 'direct'}
	{@const mailboxDomain = reporting.data?.mailbox_domain || address.split('@')[1] || ''}
	{@const orgDomain = s.reporting.report_org_domain || mailboxDomain}
	{@const domains = reporting.data?.domains ?? []}
	{@const pending = !!address && reporting.data === null && !reporting.source.error}
	{@const failed = !!address && reporting.data === null && !!reporting.source.error}

	<div class="divide-y">
		<!-- 1. The rua= address. -->
		{#snippet reachLinks()}{@render docs('mailbox', 'Mailbox & reporting setup')}{/snippet}
		{#snippet reachBody()}
			{#if address}
				Your report mailbox is <InlineCode>{address}</InlineCode>. Every domain you monitor must carry
				<InlineCode>{`rua=mailto:${address}`}</InlineCode> in its DMARC record{direct
					? ''
					: ', or an address of its own that forwards there'}.
			{:else}
				No report mailbox is set yet. Every domain you monitor must name it in the <InlineCode>rua=</InlineCode> tag of its
				DMARC record.
			{/if}
		{/snippet}
		{#snippet reachStatus()}
			{#if !address}
				{@render dot('neutral')}Not set up.
				<a href={resolve('/setup')} class="text-foreground underline underline-offset-2">Open the setup</a>
			{:else if pending}
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
							({REPORTING_STATUS_LABEL[d.state]}){/each}{more > 0 ? `, and ${more} more` : ''}.
					</span>
					{#if missing.some((d) => !d.set_up)}
						{@render fixLink('Fix the records in the setup')}
					{/if}
				{/if}
			{/if}
		{/snippet}
		{@render block('reach', 'How reports reach Marcan', reachLinks, reachBody, reachStatus)}

		<!-- 2. The mailbox domain's permission (RFC 7489 §7.1). -->
		{#snippet authLinks()}{@render docs('mailbox#central-mailbox-recommended', 'Authorization')}{/snippet}
		{#snippet authBody()}
			{#if !address}
				A mailbox on another domain than yours needs that domain's permission to receive your domains' reports.
			{:else if direct}
				The mailbox is on <InlineCode>{mailboxDomain}</InlineCode>; every domain outside <InlineCode
					>{orgDomain}</InlineCode
				>
				needs its permission, and one <InlineCode>{`*._report._dmarc.${mailboxDomain}`}</InlineCode> record with
				<InlineCode>v=DMARC1</InlineCode> covers all of them.
			{:else}
				Your domains' own addresses forward to the mailbox; an address on another domain needs that domain's permission,
				a
				<InlineCode>{'<domain>._report._dmarc.<its domain>'}</InlineCode> record with <InlineCode>v=DMARC1</InlineCode>.
			{/if}
		{/snippet}
		{#snippet authStatus()}
			{@const refused = domains.filter((d) => d.state === 'not_authorized')}
			{@const auth = reporting.data?.mailbox_authorization ?? null}
			{#if pending}
				{@render statusSkeleton()}
			{:else if failed}
				{@render checkFailed()}
			{:else if direct && auth}
				{@const label = MAILBOX_AUTHORIZATION_LABEL[auth.state]}
				{#if label === null}
					<span class="text-muted-foreground"
						>{@render dot('neutral')}Not needed: every domain is under <InlineCode>{orgDomain}</InlineCode>.</span
					>
				{:else}
					{@render dot(auth.status)}{label}{refused.length > 0
						? `: ${plural(refused.length, 'domain')} without permission`
						: ''}.
					{#if auth.state === 'no_wildcard' || auth.state === 'check_failed'}
						{@render fixLink('Publish it in the setup')}
					{/if}
				{/if}
			{:else if !direct && domains.length > 0}
				{#if refused.length > 0}
					{@render dot('fail')}Authorization missing for {plural(refused.length, 'domain')}.
					{@render fixLink('Fix it in the setup')}
				{:else}
					<span class="text-muted-foreground">{@render dot('neutral')}No domain is missing it.</span>
				{/if}
			{/if}
		{/snippet}
		{@render block(
			'permission',
			'Permission of the mailbox domain',
			authLinks,
			authBody,
			address ? authStatus : undefined
		)}

		<!-- 3. Reading and judging. -->
		{#snippet readLinks()}{@render docs('how-it-works', 'How it works')}{/snippet}
		{#snippet readBody()}
			Receivers send one report per domain and day; Marcan reads the mailbox and never changes it. Only SPF or DKIM that
			passes aligned with the From domain counts for DMARC. Each message ends as a DMARC pass, blocked (quarantined or
			rejected), forwarded (failed in transit, trusted by the receiver) or a delivered failure.
		{/snippet}
		{#snippet readStatus()}
			{#if pipeline}
				<span class="inline-flex flex-wrap gap-x-4 gap-y-1">
					<span
						>{@render dot(pipeline.mailbox.tone)}Mailbox: {pipeline.mailbox.label}{pipeline.mailbox.when && !importing
							? `, ${pipeline.mailbox.when}`
							: ''}</span
					>
					<span
						>{@render dot(pipeline.reports.tone)}Latest report: {pipeline.reports.label}{pipeline.reports.when
							? `, ${pipeline.reports.when}`
							: ''}</span
					>
				</span>
			{:else}
				{@render statusSkeleton()}
			{/if}
		{/snippet}
		{@render block('reads', 'What Marcan reads and judges', readLinks, readBody, readStatus)}

		<!-- 4. Enforcement. -->
		{#snippet enforceLinks()}{@render docs('readiness', 'Policy readiness')}{/snippet}
		{#snippet enforceBody()}
			A domain goes from <InlineCode>p=none</InlineCode>, which only monitors, to <InlineCode>p=quarantine</InlineCode>,
			which sends failing mail to spam, and then to <InlineCode>p=reject</InlineCode>, which refuses it. Policy
			readiness on each domain's page says when it is ready for the next step, from the last {rules?.readiness_window_days ??
				'…'}
			days of reports.
		{/snippet}
		{#snippet enforceStatus()}
			{#if guide.data}
				{@const p = guide.data.policies}
				<span class="text-muted-foreground">
					{plural(p.monitor, 'domain')} at <InlineCode>p=none</InlineCode>, {p.quarantine} at
					<InlineCode>p=quarantine</InlineCode>, {p.reject} at <InlineCode>p=reject</InlineCode>{p.unknown > 0
						? `, ${p.unknown} without a policy`
						: ''}.
				</span>
			{:else if guide.error}
				<span class="text-muted-foreground">{@render dot('neutral')}Policies failed to load.</span>
				<button type="button" class="text-foreground underline underline-offset-2" onclick={() => guide.reload()}>
					Retry
				</button>
			{:else}
				{@render statusSkeleton()}
			{/if}
		{/snippet}
		{@render block('enforcement', 'Enforcement', enforceLinks, enforceBody, enforceStatus)}

		<!-- 5. What Marcan sends. -->
		{#snippet sendLinks()}{@render docs('alerts', 'Alerts')}{@render docs('client-reports', 'Client reports')}{/snippet}
		{#snippet sendBody()}
			{@const sending = s.sending.enabled}
			<span class="block">
				<span class="text-foreground font-medium">Alerts</span> email you when a domain's records, policy, failures or
				reports change:
				{#if !s.alerts.enabled}off{:else if !sending}on, but sending is off{:else}on{/if}
				(<SettingsLink to="alerts">Settings → Alerts</SettingsLink>).
			</span>
			<span class="block">
				<span class="text-foreground font-medium">Client reports</span> send each organization a monthly email about its
				domains:
				{#if !s.client_reports.enabled}off{:else if !sending}on, but sending is off{:else}on{s.client_reports
						.next_send_at !== null
						? `, next on ${formatDay(s.client_reports.next_send_at)}`
						: ''}{/if}
				(<SettingsLink to="client-reports">Settings → Client reports</SettingsLink>).
			</span>
		{/snippet}
		{@render block('sending', 'Alerts and client reports', sendLinks, sendBody)}
	</div>

	<div class="flex flex-wrap gap-2 border-t pt-4">
		<Button href={resolve('/setup')} variant="outline" size="sm">Run setup again</Button>
		<Button href={DOCS_URL} target="_blank" rel="noopener" variant="outline" size="sm">
			All docs
			<ExternalLinkIcon />
		</Button>
	</div>
{/snippet}
