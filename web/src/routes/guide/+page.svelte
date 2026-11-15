<script lang="ts">
	import { resolve } from '$app/paths';
	import { getGuide } from '$lib/api';
	import type { SettingsResponse } from '$lib/types/SettingsResponse';
	import type { Status } from '$lib/types/Status';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import DocsLink from '$lib/components/guide/docs-link.svelte';
	import GuideBlock from '$lib/components/guide/guide-block.svelte';
	import GuideReporting from '$lib/components/guide/guide-reporting.svelte';
	import InlineCode from '$lib/components/inline-code.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import { settingsTitle } from '$lib/settings-nav';
	import { ReportingSetupState } from '$lib/components/reporting-setup/reporting-setup-state.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { formatDay, plural } from '$lib/format';
	import { DOCS_URL } from '$lib/nav';
	import { pipelineStatus } from '$lib/pipeline';
	import { reportAddressKey } from '$lib/report-address';
	import { meta } from '$lib/state/meta.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { statusDot } from '$lib/tokens';
	import { cn } from '$lib/utils.js';

	/**
	 * Tools → Guide: how Marcan works, block by block, with this installation's values and where
	 * each part stands, read from what the server already answers (the reporting setup the setup
	 * checks, the settings, the health, `/api/meta`) plus the policy counts (`/api/guide`).
	 * Read-only; the setup and the docs go deeper.
	 */

	const loaded = resource({
		key: () => 'guide',
		load: () => Promise.all([settings.load(), meta.load().catch(() => null)]).then(([s]) => s)
	});
	const guide = resource({ key: () => 'guide-policies', load: getGuide, reloadOn: () => systemHealth.dnsTick });
	/** The same check as the setup's (the addresses, the domains' records, the address domains' permission). */
	const addressKey = $derived(reportAddressKey(settings.current));
	const reporting = new ReportingSetupState({ reloadOn: () => addressKey });

	const health = $derived(systemHealth.current);
	const pipeline = $derived(health ? pipelineStatus(health) : null);
	const importing = $derived(!!health?.pass_progress);
	const rules = $derived(meta.current);
</script>

{#snippet dot(tone: Status)}
	<span class={cn('mr-1.5 inline-block size-2 shrink-0 rounded-full align-middle', statusDot[tone])} aria-hidden="true"
	></span>
{/snippet}

{#snippet statusSkeleton()}
	<Skeleton class="h-4 w-56" />
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
	<div class="divide-y">
		<GuideReporting {reporting} {importing} />

		<!-- 3. Reading and judging. -->
		{#snippet readLinks()}<DocsLink path="how-it-works" label="How it works" />{/snippet}
		{#snippet readBody()}
			Receivers send one report per domain and day; Marcan reads each receiving account's mailbox and never changes it.
			Only SPF or DKIM that passes aligned with the From domain counts for DMARC. Each message ends as a DMARC pass,
			blocked (quarantined or rejected), forwarded (failed in transit, trusted by the receiver) or a delivered failure.
		{/snippet}
		{#snippet readStatus()}
			{#if pipeline}
				<span class="inline-flex flex-wrap gap-x-4 gap-y-1">
					<span
						>{@render dot(pipeline.mailbox.tone)}Accounts: {pipeline.mailbox.label}{pipeline.mailbox.when && !importing
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
		<GuideBlock id="reads" title="What Marcan reads and judges" links={readLinks} body={readBody} status={readStatus} />

		<!-- 4. Enforcement. -->
		{#snippet enforceLinks()}<DocsLink path="readiness" label="Enforcement" />{/snippet}
		{#snippet enforceBody()}
			A domain goes from <InlineCode>p=none</InlineCode>, which only monitors, to <InlineCode>p=quarantine</InlineCode>,
			which sends failing mail to spam, and then to <InlineCode>p=reject</InlineCode>, which refuses it. The Enforcement
			tab on each domain's page says when it is ready for the next step, from the last {rules?.readiness_window_days ??
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
		<GuideBlock id="enforcement" title="Enforcement" links={enforceLinks} body={enforceBody} status={enforceStatus} />

		<!-- 5. What Marcan sends. -->
		{#snippet sendLinks()}<DocsLink path="alerts" label="Alerts" /><DocsLink
				path="client-reports"
				label="Client reports"
			/>{/snippet}
		{#snippet sendBody()}
			{@const sending = s.sending.enabled}
			<!-- On but not sent: the switch to flip is Sending's. -->
			{@const alertsTo = s.alerts.enabled && !sending ? 'sending' : 'alerts'}
			{@const reportsTo = s.client_reports.enabled && !sending ? 'sending' : 'client-reports'}
			<span class="block">
				<span class="text-foreground font-medium">Alerts</span> email you when a domain's records, policy, failures or
				reports change:
				{#if !s.alerts.enabled}off{:else if !sending}on, but sending is off{:else}on{/if}
				(<SettingsLink to={alertsTo}>Settings → {settingsTitle(alertsTo)}</SettingsLink>).
			</span>
			<span class="block">
				<span class="text-foreground font-medium">Client reports</span> send each organization a monthly email about its
				domains:
				{#if !s.client_reports.enabled}off{:else if !sending}on, but sending is off{:else}on{s.client_reports
						.next_send_at !== null
						? `, next on ${formatDay(s.client_reports.next_send_at)}`
						: ''}{/if}
				(<SettingsLink to={reportsTo}>Settings → {settingsTitle(reportsTo)}</SettingsLink>).
			</span>
		{/snippet}
		<GuideBlock id="sending" title="Alerts and client reports" links={sendLinks} body={sendBody} />
	</div>

	<div class="flex flex-wrap gap-2 border-t pt-4">
		<Button href={resolve('/setup')} variant="outline" size="sm">Run setup again</Button>
		<Button href={DOCS_URL} target="_blank" rel="noopener" variant="outline" size="sm">
			All docs
			<ExternalLinkIcon />
		</Button>
	</div>
{/snippet}
