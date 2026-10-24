<script lang="ts">
	import { resolve } from '$app/paths';
	import { settingsHref } from '$lib/settings-nav';
	import { getAlerts } from '$lib/api';
	import type { AlertItem } from '$lib/types/AlertItem';
	import type { Page } from '$lib/types/Page';
	import DataTable from '$lib/components/data-table.svelte';
	import type { ColumnDef } from '$lib/components/data-table';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Loaded from '$lib/components/loaded.svelte';
	import PageHeader from '$lib/components/page-header.svelte';
	import StatusBadge from '$lib/components/status-badge.svelte';
	import { formatDateTime } from '$lib/format';
	import { meta } from '$lib/state/meta.svelte';
	import { catalogueSummary, catalogueTrigger } from '$lib/wording/alert-catalogue';
	import { filters } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import BellIcon from '@lucide/svelte/icons/bell';
	import AlertText from './alert-text.svelte';

	const PAGE_SIZE = 50;
	/** The Domain cell of an event about the installation (a newer release), which has no domain. */
	const INSTALLATION = 'Marcan';

	/** Newest first, always: the list has no other order. */
	const list = new ServerList('');
	const scopeKey = $derived(`${filters.organizationId ?? ''}:${filters.domainId ?? ''}:${filters.since}`);

	const alerts = resource({
		key: () => scopeKey,
		load: () =>
			getAlerts({
				organization_id: filters.organizationId,
				domain_id: filters.domainId,
				since: filters.since,
				limit: PAGE_SIZE,
				offset: list.offsetFor(scopeKey)
			}),
		reloadOn: () => list.offsetFor(scopeKey),
		markUpdated: true
	});

	$effect(() => {
		settings.load().catch(() => {});
		meta.load().catch(() => {});
	});
	/** "Alert kinds" in the alert settings: every kind of alert and its trigger. */
	const catalogueHref = settingsHref('alert-kinds');
	/** How often changes are checked, from `meta` ("…" until it has loaded). */
	const checkMinutes = $derived(meta.current?.alert_interval_minutes ?? '…');
	/**
	 * Where the emails stand, for the line under the title (nothing until the settings are
	 * known). The list holds every change either way: the alert settings govern the emails only.
	 */
	const mailNote = $derived.by(() => {
		const s = settings.current;
		if (!s) return null;
		if (!s.alerts.enabled) return 'Alert emails are off.';
		if (!s.sending.enabled) return 'Alert emails are on, but sending is off: nothing is mailed.';
		const to = `Changes are emailed to ${s.alerts.recipients.join(', ')}`;
		const catalogue = meta.current?.alert_catalogue;
		if (!catalogue || s.alerts.muted_kinds.length === 0) return `${to}.`;
		return `${to} (alert kinds: ${catalogueSummary(catalogue, s.alerts.muted_kinds)}).`;
	});

	const columns: ColumnDef<AlertItem>[] = [
		{
			id: 'time',
			header: 'Found',
			accessor: (r) => r.created_at,
			format: (_v, r) => formatDateTime(r.created_at),
			class: 'whitespace-nowrap tabular-nums'
		},
		{ id: 'domain', header: 'Domain', accessor: (r) => r.domain ?? INSTALLATION },
		{ id: 'change', header: 'Change', accessor: (r) => r.event.kind, class: 'w-full min-w-72 whitespace-normal' }
	];
</script>

<PageHeader
	title="Alerts"
	description={`What changed about your domains, newest first. Every change is listed here, whether alert emails are on or not; a state that stays the same is never listed twice. Changes are checked every ${checkMinutes} minutes, after the DNS check.`}
>
	{#snippet titleAddon()}
		<StatusBadge tone="warn" class="text-[10px]">Experimental</StatusBadge>
	{/snippet}
	{#snippet below()}
		{#if mailNote}
			<p class="text-muted-foreground text-sm">
				{mailNote}
				<a href={settingsHref('alerts')} class="text-foreground underline underline-offset-2">Alert email settings</a>
			</p>
		{/if}
	{/snippet}
</PageHeader>

{#snippet loaded(page: Page<AlertItem>)}
	{#if page.total === 0}
		<EmptyState
			icon={BellIcon}
			headline="No changes in this scope"
			description="Marcan compares every domain with how it last stood and lists what changed here. A domain's first check only records where it stands."
		>
			{#snippet action()}
				<a href={catalogueHref} class="text-sm underline underline-offset-2">What creates an alert</a>
			{/snippet}
		</EmptyState>
	{:else}
		<DataTable {columns} data={page.items} getRowId={(r) => String(r.id)} dense server={list.table(scopeKey, page)}>
			{#snippet cell({ column, row })}
				{#if column.id === 'domain'}
					{#if row.domain_id === null}
						<span class="text-muted-foreground">{INSTALLATION}</span>
					{:else}
						<a href={resolve(`/domains/${row.domain_id}`)} class="hover:underline">{row.domain}</a>
					{/if}
				{:else if column.id === 'change'}
					<span class="block min-w-0" title={catalogueTrigger(meta.current?.alert_catalogue, row.catalogue_id)}>
						<AlertText item={row} />
					</span>
				{:else}
					{formatDateTime(row.created_at)}
				{/if}
			{/snippet}
		</DataTable>
	{/if}
{/snippet}

{#snippet loading()}
	<DataTable {columns} data={[]} getRowId={(r) => String(r.id)} loading dense />
{/snippet}

<Loaded source={alerts} errorTitle="Alerts failed to load" data={loaded} {loading} />
