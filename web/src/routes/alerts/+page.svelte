<script lang="ts">
	import { resolve } from '$app/paths';
	import { settingsHref, settingsTitle } from '$lib/settings-nav';
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
	import { catalogueTrigger } from '$lib/wording/alert-catalogue';
	import { filters } from '$lib/state/filters.svelte';
	import { resource } from '$lib/state/resource.svelte';
	import { ServerList } from '$lib/state/server-list.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { DOCS_URL } from '$lib/nav';
	import BellIcon from '@lucide/svelte/icons/bell';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import AlertText from './alert-text.svelte';

	const PAGE_SIZE = 50;
	/** The Domain cell of an event about the installation (a newer release, an account): empty. */
	const NO_DOMAIN = '—';

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
	/**
	 * Where the alert emails stand, for the status line in the header (nothing until the settings
	 * are known). The list holds every change either way: the alert settings govern the emails only.
	 */
	const mailState = $derived.by(() => {
		const s = settings.current;
		if (!s) return null;
		if (!s.alerts.enabled) return { text: 'Alert emails are off', fix: 'alerts' as const };
		// The switch to flip is Sending's.
		if (!s.sending.enabled) return { text: 'Alert emails are on, sending is off', fix: 'sending' as const };
		return { text: 'Alert emails are on', fix: 'alerts' as const };
	});

	const columns: ColumnDef<AlertItem>[] = [
		{
			id: 'time',
			header: 'Found',
			accessor: (r) => r.created_at,
			format: (_v, r) => formatDateTime(r.created_at),
			class: 'whitespace-nowrap tabular-nums'
		},
		{ id: 'domain', header: 'Domain', accessor: (r) => r.domain ?? NO_DOMAIN },
		{ id: 'severity', header: 'Severity', accessor: (r) => r.severity_label, class: 'whitespace-nowrap' },
		{ id: 'change', header: 'Change', accessor: (r) => r.event.kind, class: 'w-full min-w-72 whitespace-normal' }
	];
</script>

<PageHeader title="Alerts" description="Every change to your domains, newest first.">
	{#snippet titleAddon()}
		<StatusBadge tone="warn" class="text-[10px]">Experimental</StatusBadge>
	{/snippet}
	{#snippet actions()}
		{#if mailState}
			<p class="text-muted-foreground text-sm">
				{mailState.text}
				<span aria-hidden="true">·</span>
				<a href={settingsHref(mailState.fix)} class="hover:text-foreground underline underline-offset-2"
					>Settings → {settingsTitle(mailState.fix)}</a
				>
			</p>
		{/if}
		<Button variant="outline" size="sm" href="{DOCS_URL}alerts" target="_blank" rel="noopener">
			<BookOpenIcon />
			Docs
		</Button>
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
						<span class="text-muted-foreground">{NO_DOMAIN}</span>
					{:else}
						<a href={resolve(`/domains/${row.domain_id}`)} class="hover:underline">{row.domain}</a>
					{/if}
				{:else if column.id === 'severity'}
					<StatusBadge tone={row.event.severity}>{row.severity_label}</StatusBadge>
				{:else if column.id === 'change'}
					<span class="block min-w-0" title={catalogueTrigger(meta.current?.alert_catalogue, row.catalogue_id)}>
						<AlertText item={row} badge={false} />
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
