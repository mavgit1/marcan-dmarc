<script lang="ts">
	import { getSenders } from '$lib/api';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import type { Page } from '$lib/types/Page';
	import type { ParkedReadiness } from '$lib/types/ParkedReadiness';
	import type { PolicyStep } from '$lib/types/PolicyStep';
	import type { SenderSummary } from '$lib/types/SenderSummary';
	import type { WithIdentity } from '$lib/types/WithIdentity';
	import BusyButton from '$lib/components/busy-button.svelte';
	import DataTable from '$lib/components/data-table.svelte';
	import { defaultCellText, type ColumnDef } from '$lib/components/data-table';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import { POLICY_SOURCE_TEXT } from '$lib/wording/policy-source';
	import SenderCell from '$lib/components/sender-cell.svelte';
	import SenderName from '$lib/components/sender-name.svelte';
	import { formatDate, formatInt, plural } from '$lib/format';
	import { displayName } from '$lib/identity';
	import { meta } from '$lib/state/meta.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import { statusFg, statusTextPlain, statusTextStrong } from '$lib/tokens';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleDashedIcon from '@lucide/svelte/icons/circle-dashed';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';

	/**
	 * The domain's next policy step with what it was judged from: the server's verdict, the
	 * window's figures and each rule's answer (`policy_readiness`, worded by the server: `lead`,
	 * `detail`, `evidence`, `steps`, `blocked_note`, `parked`), the senders whose failing mail the
	 * next step acts on, and the records to publish (`steps`, each `record` written by the server
	 * into the stored DNS check's record, else a new one). Nothing is checked, judged or worded from the state here.
	 *
	 * A parked domain with a policy has its own layout: the instruction and the record first,
	 * then every message of the window (a parked domain should send none). `onPark` marks the
	 * domain parked (the page's own handler, which reloads the overview); `parking` while it runs.
	 */
	let {
		overview,
		onPark,
		parking = false
	}: { overview: DomainOverview; onPark?: () => void; parking?: boolean } = $props();

	type Sender = WithIdentity<SenderSummary>;

	/** What receivers are asked to do with failing mail under each step. */
	const STEP_EFFECT = {
		quarantine: 'deliver it to spam',
		reject: 'refuse it'
	} as const satisfies Record<PolicyStep, string>;

	const PAGE_SIZE = 50;

	$effect(() => {
		meta.load().catch(() => {});
	});
	/** The readiness window's length in the help text ("…" until `meta` has loaded). */
	const windowDays = $derived(meta.current?.readiness_window_days ?? '…');

	const readiness = $derived(overview.policy_readiness);
	const figures = $derived(readiness.window);
	const next = $derived(readiness.steps[0] ?? null);
	/** The parked layout's words; `null` for every other domain. */
	const parked = $derived(readiness.parked);

	// The window's failing senders, in the window the verdict used (`window.since`, read when
	// loading so the list does not restart on every refresh); for a parked domain every sender
	// of the window, most mail first.
	const failingList = new ServerList('-failed_messages');
	const parkedList = new ServerList('-total_messages');
	const list = $derived(parked ? parkedList : failingList);
	const showSenders = $derived(parked ? figures.messages > 0 : !!next);
	const domainId = $derived(overview.id);
	const scopeKey = $derived(`enforcement:${domainId}:${parked ? 'all' : 'failing'}`);
	const senders = resource({
		key: () => scopeKey,
		load: () =>
			showSenders
				? getSenders({
						domain_id: domainId,
						since: String(figures.since),
						...(parked ? {} : { failing: true }),
						sort: list.sort,
						limit: PAGE_SIZE,
						offset: list.offsetFor(scopeKey)
					})
				: null,
		reloadOn: () => [list.sort, list.offsetFor(scopeKey), showSenders]
	});

	const columns: ColumnDef<Sender>[] = [
		{ id: 'sender', header: 'Sender', accessor: (r) => displayName(r.identity) },
		{ id: 'ip', header: 'Source IP', accessor: (r) => r.source_ip, sortKey: 'source_ip' },
		{
			id: 'failed',
			header: 'Failed',
			accessor: (r) => r.failed_messages,
			numeric: true,
			sortKey: 'failed_messages',
			format: (v) => formatInt(Number(v ?? 0))
		},
		{
			id: 'messages',
			header: 'Messages',
			accessor: (r) => r.total_messages,
			numeric: true,
			sortKey: 'total_messages',
			format: (v) => formatInt(Number(v ?? 0))
		},
		{
			id: 'last_seen',
			header: 'Last seen',
			accessor: (r) => r.last_seen,
			sortKey: 'last_seen',
			format: (v) => formatDate(Number(v))
		}
	];
</script>

{#if parked}
	{@render parkedLayout(parked)}
{:else}
	<div class="space-y-6">
		<div class="max-w-2xl space-y-1 text-sm">
			<p>
				<span class={statusTextStrong[readiness.status]}>{readiness.lead}:</span>
				<span class="text-muted-foreground">{readiness.detail}</span>
			</p>
			{#if readiness.blocked_note}
				<p class="text-muted-foreground">{readiness.blocked_note}</p>
			{/if}
			{#if onPark && readiness.state.kind === 'not_ready' && readiness.state.reason.kind === 'too_little_mail'}
				<div class="flex flex-wrap items-center gap-x-3 gap-y-1.5 pt-1">
					<span class="text-muted-foreground">Sends no mail at all?</span>
					<BusyButton variant="outline" size="sm" busy={parking} busyLabel="Marking…" onclick={onPark}>
						Mark as parked
					</BusyButton>
				</div>
			{/if}
		</div>

		<section class="space-y-2">
			<h2 class="text-sm font-medium">What the verdict is based on</h2>
			<dl class="divide-y rounded-lg border text-sm">
				{@render currentPolicy()}
				{#each readiness.evidence as row (row.label)}
					<div class="grid gap-1 px-4 py-2.5 sm:grid-cols-[12rem_1fr]">
						<dt class="text-muted-foreground">{row.label}</dt>
						<dd class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
							<span class="space-y-0.5">
								<span class="block tabular-nums">
									{row.value}{#if row.rule}<span class="text-muted-foreground">, {row.rule}</span>{/if}
								</span>
								{#if row.hint}
									<span class="text-muted-foreground block text-xs">{row.hint}</span>
								{/if}
							</span>
							{#if row.met}
								<span class={['inline-flex items-center gap-1 text-xs', statusFg.pass]}>
									<CircleCheckIcon class="size-3.5" />Met
								</span>
							{:else}
								<span class="text-muted-foreground inline-flex items-center gap-1 text-xs">
									<CircleDashedIcon class="size-3.5" />Not yet
								</span>
							{/if}
						</dd>
					</div>
				{/each}
			</dl>
		</section>

		{#if next}
			<section class="space-y-2">
				<h2 class="text-sm font-medium">Who would be affected</h2>
				<p class="text-muted-foreground max-w-2xl text-sm">
					Senders whose mail failed DMARC in the last {windowDays} days ({plural(figures.failed, 'message')}). Under {next.policy}
					receivers are asked to {STEP_EFFECT[next.p]}.
					{figures.forwarded_failures > 0
						? `${plural(figures.forwarded_failures, 'forwarded message')} left out: receivers delivered them because they trusted the forwarder.`
						: 'Forwarded mail is left out: receivers deliver it when they trust the forwarder.'}
				</p>
				<Loaded source={senders} errorTitle="Senders failed to load" data={affected} loading={affectedLoading} />
			</section>
		{/if}

		{#if readiness.steps.length > 0 || overview.dmarc_record}
			<section class="space-y-3">
				<h2 class="text-sm font-medium">What to publish</h2>
				<p class="text-muted-foreground max-w-2xl text-sm">
					The TXT record at <code class="font-mono text-xs">_dmarc.{overview.domain}</code>.
					{#if !overview.dmarc_record && readiness.steps.length > 0}
						The last DNS check stored no single DMARC record, so these start from a new one; keep any tags yours already
						has.
					{/if}
				</p>
				{#if overview.dmarc_record}
					<DnsRawRecord
						value={overview.dmarc_record}
						label="Current record"
						emphasis={readiness.steps.length > 0 ? 'secondary' : undefined}
					/>
				{/if}
				{#if readiness.steps.length > 0}
					<ol class="space-y-3">
						{#each readiness.steps as step, i (i)}
							<li>
								<DnsRawRecord
									value={step.record}
									label={`${step.when}: ${step.policy}`}
									emphasis={step.primary ? 'primary' : 'secondary'}
								/>
							</li>
						{/each}
					</ol>
				{/if}
			</section>
		{/if}
	</div>
{/if}

{#snippet currentPolicy()}
	<div class="grid gap-1 px-4 py-2.5 sm:grid-cols-[12rem_1fr]">
		<dt class="text-muted-foreground">Current policy</dt>
		<dd class="flex flex-wrap items-center gap-x-2 gap-y-1">
			<PolicyBadge
				enforcement={overview.assessment.enforcement}
				policy={overview.policy?.p}
				source={overview.policy_source}
			/>
			<span class="text-muted-foreground text-xs">{POLICY_SOURCE_TEXT[overview.policy_source].title}</span>
		</dd>
	</div>
{/snippet}

{#snippet parkedLayout(verdict: ParkedReadiness)}
	<div class="space-y-6">
		<div class="max-w-2xl space-y-3">
			<p class="text-sm">
				<span class={['font-medium', statusTextPlain[readiness.status]]}>{verdict.lead}</span>
				<span class="text-muted-foreground">{verdict.detail}</span>
			</p>
			{#if next}
				<DnsRawRecord value={next.record} label={`${next.when}: ${next.policy}`} emphasis="primary" />
			{:else if overview.dmarc_record}
				<DnsRawRecord value={overview.dmarc_record} label="Current record" />
			{/if}
			<p class="text-muted-foreground text-xs">
				The TXT record at <code class="font-mono">_dmarc.{overview.domain}</code>.
			</p>
		</div>

		<section class="space-y-2">
			<h2 class="text-sm font-medium">Mail sent in this domain's name</h2>
			<p class="text-muted-foreground max-w-2xl text-sm">{verdict.mail_line}</p>
			{#if showSenders}
				<Loaded source={senders} errorTitle="Senders failed to load" data={affected} loading={affectedLoading} />
			{/if}
		</section>
	</div>
{/snippet}

{#snippet affected(page: Page<Sender>)}
	{#if page.total === 0}
		<EmptyState
			icon={ShieldCheckIcon}
			headline="No failing senders"
			description={`No mail failed DMARC in the last ${windowDays} days, forwarded mail aside.`}
		/>
	{:else}
		<DataTable
			{columns}
			data={page.items}
			getRowId={(r) => r.source_ip}
			getRowHref={(r) => `/senders/${encodeURIComponent(r.source_ip)}`}
			dense
			server={list.table(scopeKey, page)}
		>
			{#snippet cell({ column, row })}
				{#if column.id === 'sender'}
					<SenderName identity={row.identity} />
				{:else if column.id === 'ip'}
					<SenderCell ip={row.source_ip} />
				{:else}
					{defaultCellText(column, row)}
				{/if}
			{/snippet}
		</DataTable>
	{/if}
{/snippet}

{#snippet affectedLoading()}
	<DataTable {columns} data={[]} getRowId={(r) => r.source_ip} loading dense />
{/snippet}
