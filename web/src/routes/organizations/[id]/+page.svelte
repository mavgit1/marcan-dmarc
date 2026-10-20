<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { deleteOrganization, getOrganization } from '$lib/api';
	import { isNotFound } from '$lib/http';
	import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
	import AddDomains from '$lib/components/add-domains.svelte';
	import ConfirmDialog from '$lib/components/confirm-dialog.svelte';
	import DomainPortfolio from '$lib/components/domain-portfolio.svelte';
	import ErrorState from '$lib/components/error-state.svelte';
	import ExportDialog from '$lib/components/export-dialog.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import NotFoundState from '$lib/components/not-found-state.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import { Action } from '$lib/components/action.svelte';
	import { plural } from '$lib/format';
	import { resource } from '$lib/state/resource.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { Button, buttonVariants } from '$lib/components/ui/button/index.js';
	import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { filters } from '$lib/state/filters.svelte';
	import { pageMeta } from '$lib/state/page-meta.svelte';
	import { refresh } from '$lib/state/refresh.svelte';
	import ArrowRightLeftIcon from '@lucide/svelte/icons/arrow-right-left';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import XIcon from '@lucide/svelte/icons/x';
	import AssignDomainForm from './assign-domain-form.svelte';
	import ClientReportLine from './client-report-line.svelte';
	import RenameOrganizationForm from './rename-organization-form.svelte';

	const organizationId = $derived(Number(page.params.id));

	/** The one header form or panel open at a time. */
	let open = $state<'add' | 'assign' | 'rename' | 'delete' | null>(null);
	const del = new Action();

	function onDelete(id: number) {
		void del.run(async () => {
			await deleteOrganization(id);
			await goto(resolve('/organizations'));
			refresh.refresh();
		});
	}

	const validId = $derived(Number.isInteger(organizationId) && organizationId > 0);

	/**
	 * Keyed on the id: a refresh keeps the organization on screen, so open forms (and their
	 * per-domain results) are not torn down.
	 */
	const organization = resource({
		key: () => String(organizationId),
		load: () => (validId ? getOrganization(organizationId) : null)
	});
	const notFound = $derived(!validId || isNotFound(organization.error));

	$effect(() => {
		pageMeta.entityName = notFound ? null : (organization.data?.name ?? null);
	});
	$effect(() => () => {
		pageMeta.entityName = null;
	});
</script>

{#if notFound}
	<PageHeader title="Organization" />
	<NotFoundState
		headline="Organization not found"
		description="No organization has this id; it may have been deleted."
		backHref={resolve('/organizations')}
		backLabel="Back to organizations"
	/>
{:else}
	<Loaded source={organization} errorTitle="Failed to load organization" {header} data={body} {loading} {error} />
{/if}

{#snippet error(err: unknown)}
	<PageHeader title="Organization" />
	<ErrorState title="Failed to load organization" error={err} onRetry={() => refresh.refresh()} />
{/snippet}

{#snippet loading()}
	<div class="mb-6 space-y-2">
		<Skeleton class="w-24" />
		<Skeleton class="h-7 w-48" />
	</div>
{/snippet}

{#snippet header(c: OrganizationSummary)}
	<PageHeader title={c.name}>
		{#snippet titleAddon()}
			{#if c.is_default}
				<StatusBadge tone="neutral">
					<span title="Domains discovered in reports are placed here">Default</span>
				</StatusBadge>
			{/if}
		{/snippet}
		{#snippet actions()}
			<div class="flex flex-wrap items-center gap-2">
				{#if open === 'rename'}
					<RenameOrganizationForm organization={c} onClose={() => (open = null)} />
				{:else if open === 'assign'}
					<AssignDomainForm organizationId={c.id} onClose={() => (open = null)} />
				{:else}
					<Button size="sm" class="h-8 gap-1.5 text-xs" onclick={() => (open = 'add')}>
						<PlusIcon class="size-3.5" />
						Add domains
					</Button>
					<Button size="sm" variant="outline" class="h-8 gap-1.5 text-xs" onclick={() => (open = 'assign')}>
						<ArrowRightLeftIcon class="size-3.5" />
						Assign domain
					</Button>
					<ExportDialog
						datasets={['domains', 'senders', 'auth-issues', 'dns-findings']}
						scope={{ organization_id: c.id, since: filters.since }}
						organizationName={c.name}
					/>
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							class={buttonVariants({ variant: 'outline', size: 'icon-sm' })}
							aria-label="More organization actions"
							title="More actions"
						>
							<EllipsisIcon class="size-4" />
						</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end">
							<DropdownMenu.Item onclick={() => (open = 'rename')}>
								<PencilIcon />
								Rename
							</DropdownMenu.Item>
							{#if !c.is_default}
								<DropdownMenu.Item
									variant="destructive"
									onclick={() => {
										open = 'delete';
										del.feedback = null;
									}}
								>
									<Trash2Icon />
									Delete
								</DropdownMenu.Item>
							{/if}
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				{/if}
			</div>
		{/snippet}
	</PageHeader>

	{#if open === 'add'}
		<Card.Root class="mb-6">
			<Card.Header class="pb-3">
				<div class="flex items-center justify-between gap-2">
					<Card.Title class="text-sm font-semibold">Add domains to {c.name}</Card.Title>
					<Button variant="ghost" size="icon-sm" aria-label="Close" onclick={() => (open = null)}>
						<XIcon class="size-3.5" />
					</Button>
				</div>
			</Card.Header>
			<Card.Content>
				<AddDomains organizationId={c.id} autofocus onAdded={() => refresh.refresh()} />
			</Card.Content>
		</Card.Root>
	{/if}
{/snippet}

{#snippet body(c: OrganizationSummary)}
	{#if open === 'delete' && !c.is_default}
		<ConfirmDialog
			title={`Delete ${c.name}?`}
			confirmLabel="Delete organization"
			busyLabel="Deleting…"
			action={del}
			onConfirm={() => onDelete(c.id)}
			onCancel={() => (open = null)}
		>
			<p class="text-muted-foreground text-xs">
				{c.domain_count === 1
					? 'Its domain and its reports are not deleted: they move'
					: `Its ${plural(c.domain_count, 'domain')} and their reports are not deleted: they move`}
				to the default organization.
			</p>
		</ConfirmDialog>
	{/if}
{/snippet}

<!-- Outside the header branches so it loads alongside the organization and survives refreshes. -->
{#if !notFound && !(organization.data === null && organization.error)}
	<DomainPortfolio
		{organizationId}
		showOrganization={false}
		emptyHeadline="No domains assigned yet"
		emptyDescription="Add a domain above to monitor DNS and aggregate reports for this organization."
	>
		{#snippet afterFigures()}
			{#if organization.data}
				<ClientReportLine {organizationId} organizationName={organization.data.name} />
			{/if}
		{/snippet}
	</DomainPortfolio>
{/if}
