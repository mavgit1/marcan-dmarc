<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { getDomain, ignoreDomain, updateDomain, type DomainFailureFilters } from '$lib/api';
	import { isNotFound } from '$lib/http';
	import type { DomainOverview } from '$lib/types/DomainOverview';
	import ConfirmDialog from '$lib/components/confirm-dialog.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import ExportDialog from '$lib/components/export-dialog.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import NotFoundState from '$lib/components/not-found-state.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import PolicyBadge from '$lib/components/policy-badge.svelte';
	import SendersTable from '$lib/components/senders-table.svelte';
	import { Action } from '$lib/components/action.svelte';
	import FeedbackAlert from '$lib/components/settings/feedback-alert.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { buttonVariants } from '$lib/components/ui/button/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import { filters } from '$lib/state/filters.svelte';
	import { statusTextStrong } from '$lib/tokens';
	import { pageMeta } from '$lib/state/page-meta.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import ArrowRightLeftIcon from '@lucide/svelte/icons/arrow-right-left';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import BanIcon from '@lucide/svelte/icons/ban';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import ParkingSquareIcon from '@lucide/svelte/icons/square-parking';
	import EnforcementTab from './enforcement-tab.svelte';
	import FailuresTab from './failures-tab.svelte';
	import MoveDomainForm from './move-domain-form.svelte';
	import OverviewTab from './overview-tab.svelte';
	import SectionTabs from '$lib/components/section-tabs.svelte';

	const TABS = [
		{ id: 'overview', label: 'Overview' },
		{ id: 'senders', label: 'Senders' },
		{ id: 'failures', label: 'Failures' },
		{ id: 'enforcement', label: 'Enforcement' }
	] as const;

	type TabId = (typeof TABS)[number]['id'];

	const domainId = $derived(Number(page.params.id));

	/** Filters of the Failures tab while it is open; they scope the auth-issues export. */
	let failureFilters = $state<DomainFailureFilters | null>(null);

	/** The one header form or panel open at a time. */
	let open = $state<'move' | 'ignore' | null>(null);
	const park = new Action();
	const ignore = new Action();

	function onSetParked(parked: boolean) {
		void park.run(async () => {
			await updateDomain(domainId, { parked });
			refresh.refresh();
		});
	}

	function onIgnore(organizationId: number) {
		void ignore.run(async () => {
			await ignoreDomain(domainId);
			await goto(resolve(`/organizations/${organizationId}`));
			refresh.refresh();
		});
	}

	const activeTab = $derived.by((): TabId => {
		const raw = page.url.searchParams.get('tab');
		if (raw && TABS.some((t) => t.id === raw)) return raw as TabId;
		return 'overview';
	});

	const validId = $derived(Number.isInteger(domainId) && domainId > 0);

	/**
	 * Keyed on the domain only: a refresh or a new time range keeps the last answer on screen
	 * until the next arrives, so the tabs, their filters and open rows survive; only another
	 * domain starts from a skeleton.
	 */
	const domain = resource({
		key: () => String(domainId),
		load: () => (validId ? getDomain(domainId, { since: filters.since }) : null),
		reloadOn: () => filters.since,
		markUpdated: true
	});
	const notFound = $derived(!validId || isNotFound(domain.error));

	$effect(() => {
		pageMeta.entityName = notFound ? null : (domain.data?.domain ?? null);
	});
	$effect(() => () => {
		pageMeta.entityName = null;
	});

	/** This page with tab `id` open, the other parameters kept. */
	function tabHref(id: TabId): string {
		const url = new URL(page.url);
		if (id === 'overview') {
			url.searchParams.delete('tab');
		} else {
			url.searchParams.set('tab', id);
		}
		return resolve(`${url.pathname}${url.search}${url.hash}` as '/');
	}

	function setTab(id: TabId) {
		void goto(tabHref(id), { keepFocus: true, noScroll: true });
	}
</script>

{#if notFound}
	<PageHeader title="Domain" />
	<NotFoundState
		headline="Domain not found"
		description="No monitored domain has this id; it may have been ignored."
		backHref={resolve('/domains')}
		backLabel="Back to domains"
	/>
{:else}
	<Loaded source={domain} errorTitle="Domain failed to load" {header} data={body} {loading} {error} />
{/if}

{#snippet error(err: unknown)}
	<PageHeader title="Domain" />
	<ErrorState title="Domain failed to load" error={err} onRetry={() => refresh.refresh()} />
{/snippet}

{#snippet loading()}
	<PageHeader title="Domain" />
	<div class="grid gap-3 sm:grid-cols-3">
		<Skeleton variant="block" class="h-24" />
		<Skeleton variant="block" class="h-24" />
		<Skeleton variant="block" class="h-24" />
	</div>
{/snippet}

{#snippet header(overview: DomainOverview)}
	<PageHeader title={overview.domain} description={`Organization: ${overview.organization.name}`}>
		{#snippet titleAddon()}
			{#if overview.parked}
				<StatusBadge tone="neutral">Parked</StatusBadge>
			{/if}
			<PolicyBadge
				enforcement={overview.assessment.enforcement}
				policy={overview.policy?.p}
				source={overview.policy_source}
			/>
		{/snippet}
		{#snippet below()}
			<!-- The Enforcement tab opens with the same sentence, so the header leaves it out there. -->
			{@const readiness = activeTab === 'enforcement' ? null : overview.policy_readiness.headline}
			{#if readiness}
				<p class="max-w-2xl text-sm">
					<span class={statusTextStrong[overview.policy_readiness.status]}>{readiness}</span>
					<span class="text-muted-foreground" aria-hidden="true">·</span>
					<a
						href={tabHref('enforcement')}
						class="text-muted-foreground hover:text-foreground underline underline-offset-2"
						onclick={(e) => {
							e.preventDefault();
							setTab('enforcement');
						}}>Details</a
					>
				</p>
			{/if}
		{/snippet}
		{#snippet actions()}
			{#if open === 'move'}
				<MoveDomainForm {domainId} organizationId={overview.organization.id} onClose={() => (open = null)} />
			{:else}
				<ExportDialog
					datasets={['senders', 'auth-issues', 'dns-findings']}
					initial={activeTab === 'failures' ? 'auth-issues' : activeTab === 'senders' ? 'senders' : undefined}
					scope={{
						domain_id: overview.id,
						since: filters.since,
						...(activeTab === 'failures' && failureFilters
							? {
									outcome: failureFilters.outcome,
									source_ip: failureFilters.source_ip
								}
							: {})
					}}
					domainName={overview.domain}
				/>
				<DropdownMenu.Root>
					<DropdownMenu.Trigger
						class={buttonVariants({ variant: 'outline', size: 'icon-sm' })}
						aria-label="More domain actions"
						title="More actions"
					>
						<EllipsisIcon class="size-4" />
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end">
						<DropdownMenu.Item onclick={() => (open = 'move')}>
							<ArrowRightLeftIcon />
							Move to organization
						</DropdownMenu.Item>
						<DropdownMenu.Item onclick={() => onSetParked(!overview.parked)}>
							<ParkingSquareIcon />
							{overview.parked ? 'Unpark' : 'Mark as parked'}
						</DropdownMenu.Item>
						<DropdownMenu.Item
							variant="destructive"
							onclick={() => {
								open = 'ignore';
								ignore.feedback = null;
							}}
						>
							<EyeOffIcon />
							Ignore domain
						</DropdownMenu.Item>
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			{/if}
		{/snippet}
	</PageHeader>
{/snippet}

{#snippet body(overview: DomainOverview)}
	{#if park.feedback && !park.feedback.ok}
		<div class="mb-4">
			<FeedbackAlert feedback={park.feedback} />
		</div>
	{/if}

	{#if open === 'ignore'}
		<ConfirmDialog
			title={`Ignore ${overview.domain}?`}
			confirmLabel="Ignore domain"
			busyLabel="Ignoring…"
			action={ignore}
			onConfirm={() => onIgnore(overview.organization.id)}
			onCancel={() => (open = null)}
		>
			<ul class="text-muted-foreground space-y-0.5 pt-1 text-xs">
				<li class="flex items-center gap-1.5"><Trash2Icon class="size-3.5 shrink-0" />Its reports are deleted</li>
				<li class="flex items-center gap-1.5"><BanIcon class="size-3.5 shrink-0" />New reports for it are dropped</li>
				<li class="flex items-center gap-1.5">
					<RotateCcwIcon class="size-3.5 shrink-0" />Undo: drop <code class="font-mono">ignored</code> from its line in Settings
					→ Domains
				</li>
			</ul>
		</ConfirmDialog>
	{/if}

	<SectionTabs tabs={TABS} active={activeTab} label="Domain detail sections" onSelect={setTab} />

	{#if activeTab === 'overview'}
		<OverviewTab {overview} />
	{:else if activeTab === 'senders'}
		<SendersTable {domainId} />
	{:else if activeTab === 'failures'}
		<FailuresTab {domainId} {overview} bind:exportFilters={failureFilters} />
	{:else if activeTab === 'enforcement'}
		<EnforcementTab {overview} onPark={() => onSetParked(true)} parking={park.busy} />
	{/if}
{/snippet}
