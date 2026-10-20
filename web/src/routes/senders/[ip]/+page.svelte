<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getSender } from '$lib/api';
	import { isBadRequest, isNotFound } from '$lib/http';
	import type { SenderDetailResponse } from '$lib/types/SenderDetailResponse';
	import CopyButton from '$lib/components/copy-button.svelte';
	import EmptyState from '$lib/components/empty-state.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import NotFoundState from '$lib/components/not-found-state.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import OutcomeLegend from '$lib/components/outcome-legend.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import VolumeChart from '$lib/components/volume-chart.svelte';
	import { formatDateRange, formatInt, plural, pluralNoun } from '$lib/format';
	import { outcomeCounts } from '$lib/health';
	import { displayName, isUnconfirmedOperatorName, UNCONFIRMED_PTR_HINT } from '$lib/identity';
	import { seriesFromTimeseries } from '$lib/series';
	import { filters } from '$lib/state/filters.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { statusBorderL, statusTextPlain } from '$lib/tokens';
	import { listTokens, unique } from '$lib/text';
	import { cn } from '$lib/utils.js';
	import { dkimLine, recipientsOf, rowVerdict, spfLine } from './sender';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import InboxIcon from '@lucide/svelte/icons/inbox';

	let expandedDomainIds = $state<Set<number>>(new Set());

	function toggleDomainExpand(domainId: number) {
		const next = new Set(expandedDomainIds);
		if (next.has(domainId)) {
			next.delete(domainId);
		} else {
			next.add(domainId);
		}
		expandedDomainIds = next;
	}

	function decodeIpParam(raw: string | undefined): string {
		if (!raw) return '';
		try {
			return decodeURIComponent(raw);
		} catch {
			return raw;
		}
	}

	const ip = $derived(decodeIpParam(page.params.ip));

	/**
	 * Keyed on the IP: a refresh or another organization or range keeps the last answer (and
	 * the open rows) on screen until the next arrives.
	 */
	const detail = resource({
		key: () => ip,
		load: () => (ip ? getSender(ip, { organization_id: filters.organizationId, since: filters.since }) : null),
		reloadOn: () => [filters.organizationId, filters.since],
		markUpdated: true,
		onData: (data, previous) => {
			// Another IP starts folded, a lone domain open; later refreshes leave the rows be.
			if (previous === null) {
				expandedDomainIds = new Set(data.items.length === 1 ? [data.items[0].domain_id] : []);
			}
		}
	});
	const invalid = $derived(!ip || isBadRequest(detail.error));
	const notFound = $derived(isNotFound(detail.error));
</script>

{#snippet targetList(list: { target: string; count: number }[])}
	{#each list as t, i (t.target)}
		{#if i > 0},{' '}{/if}<span class="font-mono">{t.target}</span>
		<span class="text-muted-foreground tabular-nums">({formatInt(t.count)})</span>
	{/each}
{/snippet}

{#if invalid}
	<PageHeader title="Sender" />
	<NotFoundState
		headline="Not an IP address"
		description="Sender detail expects a source IPv4 or IPv6 address."
		backHref={resolve('/senders')}
		backLabel="Back to senders"
	/>
{:else if notFound}
	<PageHeader title={ip} />
	<NotFoundState
		headline="Sender not seen"
		description="This IP has no aggregate-report records in the current organization and time range."
		backHref={resolve('/senders')}
		backLabel="Back to senders"
	/>
{:else}
	<Loaded source={detail} errorTitle="Sender failed to load" {header} data={body} {loading} {error} />
{/if}

{#snippet error(err: unknown)}
	<PageHeader title="Sender" />
	<ErrorState title="Sender failed to load" error={err} onRetry={() => refresh.refresh()} />
{/snippet}

{#snippet loading()}
	<PageHeader title="Sender" />
	<Skeleton class="-mt-2 mb-4 h-4 w-80 max-w-full" />
	<Skeleton variant="block" class="mb-6 h-[340px]" />
	<Skeleton variant="block" class="h-40" />
{/snippet}

{#snippet header(sender: SenderDetailResponse)}
	<PageHeader title={displayName(sender.identity)} />
{/snippet}

{#snippet body(sender: SenderDetailResponse)}
	{@const identity = sender.identity}
	{@const unconfirmed = isUnconfirmedOperatorName(identity)}
	{@const chartSeries = seriesFromTimeseries(sender.timeseries)}

	<!-- Identity: neutral caption on one line (DESIGN.md: identity never carries a colour). -->
	<div class="text-muted-foreground -mt-2 mb-4 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
		<span class="text-foreground font-mono">{sender.source_ip}</span>
		<CopyButton value={sender.source_ip} label="Copy IP" />
		{#if identity.spf_domain}
			<span class="text-border">·</span>
			<span>SPF passes for <span class="text-foreground font-mono">{identity.spf_domain}</span></span>
		{/if}
		{#if identity.ptr_hostname}
			<span class="text-border">·</span>
			<span>
				Reverse DNS
				<span
					class={cn(
						'font-mono',
						unconfirmed
							? 'decoration-muted-foreground/60 underline decoration-dashed underline-offset-2'
							: 'text-foreground'
					)}
					title={unconfirmed ? UNCONFIRMED_PTR_HINT : 'Forward-confirmed reverse DNS'}
				>
					{identity.ptr_hostname}
				</span>
				{#if unconfirmed}(not confirmed){/if}
			</span>
		{/if}
		<span class="text-border">·</span>
		<span>Seen <span class="text-foreground">{formatDateRange(sender.first_seen, sender.last_seen)}</span></span>
		<span class="text-border">·</span>
		<span>Sends as <span class="text-foreground">{plural(sender.domain_count, 'domain')}</span></span>
	</div>

	<VolumeChart series={chartSeries} class="mb-6" outcomes={{ mail: sender, total: true }} />

	<section class="space-y-3">
		<h2 class="text-sm font-medium">Domains sending as</h2>

		{#if sender.items.length === 0}
			<EmptyState
				icon={InboxIcon}
				headline="No domains for this IP"
				description="No header-from domains were recorded for this sender in the current range."
			/>
		{:else}
			<div class="space-y-3">
				{#each sender.items as row (row.domain_id)}
					{@const isExpanded = expandedDomainIds.has(row.domain_id)}
					{@const c = outcomeCounts(row)}
					{@const verdict = rowVerdict(row)}
					{@const spf = spfLine(row)}
					{@const dkim = dkimLine(row)}
					{@const envelope = unique(listTokens(row.envelope_from)).sort()}
					{@const selectors = unique(listTokens(row.dkim_selectors)).sort()}
					{@const recipients = recipientsOf(row)}
					{@const listing = sender.spf_listings.find((l) => l.domain_id === row.domain_id)}
					<div class={cn('bg-card overflow-hidden rounded-lg border border-l-2', statusBorderL[verdict.tone])}>
						<button
							type="button"
							class="hover:bg-muted/30 flex w-full flex-wrap items-center justify-between gap-3 p-3.5 text-left transition-colors"
							onclick={() => toggleDomainExpand(row.domain_id)}
							aria-expanded={isExpanded}
						>
							<span class="flex min-w-0 flex-wrap items-center gap-2">
								<ChevronDownIcon
									class={cn('text-muted-foreground size-4 shrink-0 transition-transform', !isExpanded && '-rotate-90')}
								/>
								<span class="text-sm font-semibold">{row.domain}</span>
								<StatusBadge tone={verdict.tone}>{verdict.label}</StatusBadge>
							</span>
							<span class="text-muted-foreground flex items-center gap-4 text-xs">
								<span class="hidden sm:inline">{formatDateRange(row.first_seen, row.last_seen)}</span>
								<span
									><strong class="text-foreground tabular-nums">{formatInt(row.total_messages)}</strong>
									{pluralNoun(row.total_messages, 'message')}</span
								>
							</span>
						</button>

						{#if isExpanded}
							<div class="space-y-4 border-t px-4 py-3.5 text-xs">
								<OutcomeLegend mail={row} />

								<dl class="grid gap-x-6 gap-y-2 sm:grid-cols-[max-content_1fr]">
									<dt class="text-muted-foreground">SPF</dt>
									<dd class={cn('[overflow-wrap:anywhere]', statusTextPlain[spf.tone])}>
										{spf.text}
									</dd>
									{#if listing}
										<!-- Only literal ip4:/ip6: terms are matched, so the row is shown only when found:
										     a domain can still authorize the IP through a, mx or exists. -->
										<dt class="text-muted-foreground">Listed in SPF</dt>
										<dd class="[overflow-wrap:anywhere]">
											<span class="font-mono">{listing.term}</span>
											{#if listing.domain !== row.domain}
												in the record of <span class="font-mono">{listing.domain}</span>
											{/if}
										</dd>
									{/if}
									<dt class="text-muted-foreground">DKIM</dt>
									<dd class={cn('[overflow-wrap:anywhere]', statusTextPlain[dkim.tone])}>
										{dkim.text}
									</dd>
									<dt class="text-muted-foreground">DKIM selectors</dt>
									<dd class="font-mono [overflow-wrap:anywhere]">
										{selectors.length > 0 ? selectors.join(', ') : '—'}
									</dd>
									<dt class="text-muted-foreground">Envelope from</dt>
									<dd class="[overflow-wrap:anywhere]">
										{#if envelope.length === 0}
											—
										{:else}
											{#each envelope as e, i (e)}
												{#if i > 0},{' '}{/if}
												{#if e === '<>'}
													<!-- Bounces and auto-replies have no envelope sender; SPF then checks the
													     sending server's own name (HELO) instead of this domain. -->
													<span title="No envelope sender: SPF checked the sending server's name instead">
														empty (bounces / auto-replies)
													</span>
												{:else}
													<span class="font-mono">{e}</span>
												{/if}
											{/each}
										{/if}
									</dd>
									<dt class="text-muted-foreground">Sent to</dt>
									<dd class="[overflow-wrap:anywhere]">
										{#if recipients.length === 0}
											—
										{:else}
											{@render targetList(recipients.slice(0, 3))}
											{#if recipients.length > 3}
												<details class="inline">
													<summary
														class="text-muted-foreground hover:text-foreground inline cursor-pointer select-none"
													>
														+{formatInt(recipients.length - 3)} more
													</summary>
													<div class="mt-1">{@render targetList(recipients.slice(3))}</div>
												</details>
											{/if}
										{/if}
									</dd>
									<dt class="text-muted-foreground">Reported by</dt>
									<dd>
										{#if row.receivers.length === 0}
											—
										{:else}
											<ul class="space-y-0.5">
												{#each row.receivers as rec (rec.reporter)}
													<li>
														{rec.reporter}
														<span class="text-muted-foreground tabular-nums">({formatInt(rec.total_messages)})</span>
													</li>
												{/each}
											</ul>
										{/if}
									</dd>
								</dl>

								<!-- Colour on the edge only; the text is plain. The reports cannot say whose IP
								     this is, and one IP can carry both (shared providers): both readings. -->
								<div class="grid gap-3 sm:grid-cols-2">
									{#if c.forwarded > 0}
										<div
											class={cn('space-y-1 rounded-lg border border-l-2 bg-background p-3.5', statusBorderL.forward)}
										>
											<h3 class="text-sm font-semibold">Forwarded</h3>
											<p class="text-muted-foreground leading-relaxed">
												{plural(c.forwarded, 'message')} failed only because a forwarder or mailing list changed {c.forwarded ===
												1
													? 'it'
													: 'them'}; the receiver trusted the forward.
											</p>
										</div>
									{/if}
									{#if c.blocked + c.delivered > 0}
										{@const failed = c.blocked + c.delivered}
										<div class={cn('space-y-1 rounded-lg border border-l-2 bg-background p-3.5', statusBorderL.fail)}>
											<h3 class="text-sm font-semibold">If this is your mail</h3>
											<p class="text-muted-foreground leading-relaxed">
												{#if c.delivered === 0}
													{plural(failed, 'message')} failed DMARC and
													{failed === 1 ? 'was' : 'were'} quarantined or rejected.
												{:else if c.blocked === 0}
													{plural(failed, 'message')} failed DMARC and
													{failed === 1 ? 'was' : 'were'} delivered only because the policy allows it.
												{:else}
													{formatInt(failed)} messages failed DMARC: {formatInt(c.blocked)} quarantined or rejected, {formatInt(
														c.delivered
													)} delivered only because the policy allows it.
												{/if}
											</p>
										</div>
										<div
											class={cn(
												'space-y-1 rounded-lg border border-l-2 bg-background p-3.5',
												statusBorderL[row.spoofing_status]
											)}
										>
											<h3 class="text-sm font-semibold">If this is spoofing</h3>
											<p class="text-muted-foreground leading-relaxed">
												{#if c.delivered === 0}
													The policy stopped all {formatInt(c.blocked)}.
												{:else if c.blocked === 0}
													{plural(c.delivered, 'spoofed message')} {c.delivered === 1 ? 'was' : 'were'} delivered.
												{:else}
													The policy stopped {formatInt(c.blocked)}; {formatInt(c.delivered)}
													{c.delivered === 1 ? 'was' : 'were'} delivered.
												{/if}
											</p>
										</div>
									{/if}
								</div>
							</div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</section>
{/snippet}
