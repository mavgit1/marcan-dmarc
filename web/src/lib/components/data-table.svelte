<script lang="ts" generics="T">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
	import ArrowUpIcon from '@lucide/svelte/icons/arrow-up';
	import ChevronsUpDownIcon from '@lucide/svelte/icons/chevrons-up-down';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import EmptyState from '$lib/components/empty-state.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Table from '$lib/components/ui/table/index.js';
	import {
		columnAlign,
		defaultCellText,
		nextSort,
		parseSort,
		type ColumnDef,
		type DataTableCellSnippet,
		type ServerTable
	} from '$lib/components/data-table';
	import { formatInt } from '$lib/format';
	import { cn } from '$lib/utils.js';

	/**
	 * Table built on shadcn Table primitives, with empty/loading states; pass a `cell`
	 * snippet to render badges, links, or other rich cells.
	 *
	 * Without `server` it sorts and pages `data` locally (for complete lists). With
	 * `server`, `data` is one page of a server-paginated list: sortable headers are the
	 * columns with a `sortKey`, and sorting and paging are requests to the API.
	 */
	let {
		columns,
		data,
		getRowId,
		getRowHref,
		onRowClick,
		rowClass,
		cell,
		loading = false,
		dense = false,
		pageSize = 10,
		paginate = true,
		emptyHeadline = 'No rows',
		emptyDescription = 'Nothing to show for the current filters.',
		server,
		class: className
	}: {
		columns: ColumnDef<T>[];
		data: T[];
		getRowId: (row: T, index: number) => string;
		getRowHref?: (row: T) => string | undefined;
		onRowClick?: (row: T) => void;
		rowClass?: (row: T) => string | undefined;
		cell?: DataTableCellSnippet<T>;
		loading?: boolean;
		dense?: boolean;
		pageSize?: number;
		paginate?: boolean;
		emptyHeadline?: string;
		emptyDescription?: string;
		server?: ServerTable;
		class?: string;
	} = $props();

	let localSortId = $state<string | null>(null);
	let localSortDir = $state<'asc' | 'desc'>('asc');
	let pageIndex = $state(0);

	const serverSort = $derived(server ? parseSort(server.sort) : null);
	/** Column currently sorted, and its direction, whichever side sorts. */
	const sortId = $derived(serverSort ? (columns.find((c) => c.sortKey === serverSort.key)?.id ?? null) : localSortId);
	const sortDir = $derived(serverSort ? serverSort.dir : localSortDir);

	function isSortable(column: ColumnDef<T>): boolean {
		return server ? column.sortKey != null : Boolean(column.sortable);
	}

	const sorted = $derived.by(() => {
		if (server || !sortId) return data;
		const col = columns.find((c) => c.id === sortId);
		if (!col) return data;
		const copy = [...data];
		copy.sort((a, b) => compareValues(col.accessor(a), col.accessor(b)));
		return sortDir === 'desc' ? copy.reverse() : copy;
	});

	const localPaging = $derived(!server && paginate && pageSize > 0);
	const pageCount = $derived(localPaging ? Math.max(1, Math.ceil(sorted.length / pageSize)) : 1);
	const safePage = $derived(Math.min(pageIndex, pageCount - 1));
	const pageRows = $derived(localPaging ? sorted.slice(safePage * pageSize, (safePage + 1) * pageSize) : sorted);

	const clickable = $derived(Boolean(onRowClick || getRowHref));

	function compareValues(a: string | number | null | undefined, b: string | number | null | undefined): number {
		if (a == null && b == null) return 0;
		if (a == null) return -1;
		if (b == null) return 1;
		if (typeof a === 'number' && typeof b === 'number') return a - b;
		return String(a).localeCompare(String(b), undefined, { numeric: true, sensitivity: 'base' });
	}

	function toggleSort(column: ColumnDef<T>) {
		if (server) {
			if (column.sortKey) server.onSort(nextSort(server.sort, column.sortKey, Boolean(column.numeric)));
			return;
		}
		if (localSortId === column.id) {
			localSortDir = localSortDir === 'asc' ? 'desc' : 'asc';
		} else {
			localSortId = column.id;
			localSortDir = 'asc';
		}
		pageIndex = 0;
	}

	function handleRowActivate(row: T, event: MouseEvent | KeyboardEvent) {
		if (onRowClick) {
			onRowClick(row);
			return;
		}
		const href = getRowHref?.(row);
		if (!href) return;
		const url = resolve(href as '/');
		if (event instanceof MouseEvent && (event.metaKey || event.ctrlKey)) {
			window.open(url, '_blank');
			return;
		}
		void goto(url);
	}

	let container = $state<HTMLDivElement>();
	let table = $state<HTMLTableElement>();
	/** Whether the table fits its column, so it needs no sideways scroll and its header can stick. */
	let fits = $state(false);
	$effect(() => {
		if (!container || !table) return;
		const box = container;
		const inner = table;
		const measure = () => (fits = inner.offsetWidth <= box.clientWidth);
		const observer = new ResizeObserver(measure);
		observer.observe(box);
		observer.observe(inner);
		measure();
		return () => observer.disconnect();
	});

	function alignClass(align: 'left' | 'right' | 'center'): string {
		if (align === 'right') return 'text-right';
		if (align === 'center') return 'text-center';
		return 'text-left';
	}
</script>

<!--
	Tables scroll with the page. A table that fits its column has no scroll box of its own,
	so its column header sticks under the app header (`overflow-clip` keeps the rounded
	corners without making a scroll container, which would stop the sticking). One that does
	not fit (a narrow screen) scrolls sideways in its box, and its header does not stick.
-->
<div class={cn('rounded-lg border', fits ? 'overflow-clip' : 'overflow-hidden', className)}>
	<div bind:this={container} data-slot="table-container" class={cn('relative w-full', !fits && 'overflow-x-auto')}>
		<table bind:this={table} data-slot="table" class={cn('w-full caption-bottom text-sm', dense && 'text-xs')}>
			<Table.Header
				class={cn('bg-card z-10', fits && 'sticky top-(--app-header-height) shadow-[0_1px_0_var(--color-border)]')}
			>
				<Table.Row class="hover:bg-transparent">
					{#each columns as column (column.id)}
						{@const align = columnAlign(column)}
						<Table.Head
							class={cn(alignClass(align), column.numeric && 'tabular-nums', dense && 'h-8', column.headerClass)}
							aria-sort={sortId === column.id
								? sortDir === 'asc'
									? 'ascending'
									: 'descending'
								: isSortable(column)
									? 'none'
									: undefined}
						>
							{#if isSortable(column)}
								<button
									type="button"
									class={cn(
										'hover:text-foreground inline-flex items-center gap-1 focus-visible:ring-ring rounded-sm focus-visible:ring-2 focus-visible:outline-none',
										align === 'right' && 'ml-auto flex-row-reverse'
									)}
									onclick={() => toggleSort(column)}
								>
									{column.header}
									{#if sortId === column.id && sortDir === 'asc'}
										<ArrowUpIcon class="size-3" />
									{:else if sortId === column.id}
										<ArrowDownIcon class="size-3" />
									{:else}
										<ChevronsUpDownIcon class="text-muted-foreground size-3" />
									{/if}
								</button>
							{:else}
								{column.header}
							{/if}
						</Table.Head>
					{/each}
				</Table.Row>
			</Table.Header>
			<Table.Body>
				{#if loading}
					{#each Array.from({ length: 6 }) as _, i (i)}
						<Table.Row>
							{#each columns as column (column.id)}
								<Table.Cell class={dense ? 'py-1' : undefined}>
									<Skeleton class="h-4 w-full" />
								</Table.Cell>
							{/each}
						</Table.Row>
					{/each}
				{:else if pageRows.length === 0}
					<Table.Row class="hover:bg-transparent">
						<Table.Cell colspan={columns.length} class="p-0">
							<EmptyState icon={InboxIcon} headline={emptyHeadline} description={emptyDescription} />
						</Table.Cell>
					</Table.Row>
				{:else}
					{#each pageRows as row, i (`${getRowId(row, i)}__${i}`)}
						<Table.Row
							class={cn(i % 2 === 1 && 'bg-muted/30', clickable && 'hover:bg-muted/60 cursor-pointer', rowClass?.(row))}
							onclick={clickable
								? (e) => {
										if ((e.target as HTMLElement).closest('a, button')) return;
										handleRowActivate(row, e);
									}
								: undefined}
							onkeydown={clickable
								? (e) => {
										if (e.key === 'Enter' || e.key === ' ') {
											e.preventDefault();
											handleRowActivate(row, e);
										}
									}
								: undefined}
							tabindex={clickable ? 0 : undefined}
						>
							{#each columns as column (column.id)}
								{@const align = columnAlign(column)}
								<Table.Cell
									class={cn(alignClass(align), column.numeric && 'tabular-nums', dense && 'py-1', column.class)}
								>
									{#if cell}
										{@render cell({ column, row, value: column.accessor(row) })}
									{:else}
										{defaultCellText(column, row)}
									{/if}
								</Table.Cell>
							{/each}
						</Table.Row>
					{/each}
				{/if}
			</Table.Body>
		</table>
	</div>
	{#if server && !loading && server.total > server.limit}
		{@const from = server.offset + 1}
		{@const to = Math.min(server.offset + data.length, server.total)}
		<div class="flex items-center justify-between gap-2 border-t px-3 py-2">
			<p class="text-muted-foreground text-xs tabular-nums">
				{formatInt(from)}–{formatInt(to)} of {formatInt(server.total)}
			</p>
			<div class="flex items-center gap-1">
				<Button
					variant="outline"
					size="xs"
					disabled={server.offset <= 0}
					onclick={() => server.onPage(Math.max(0, server.offset - server.limit))}
				>
					Previous
				</Button>
				<Button
					variant="outline"
					size="xs"
					disabled={server.offset + server.limit >= server.total}
					onclick={() => server.onPage(server.offset + server.limit)}
				>
					Next
				</Button>
			</div>
		</div>
	{:else if localPaging && !loading && sorted.length > pageSize}
		<div class="flex items-center justify-between gap-2 border-t px-3 py-2">
			<p class="text-muted-foreground text-xs tabular-nums">
				{safePage * pageSize + 1}–{Math.min((safePage + 1) * pageSize, sorted.length)} of {sorted.length}
			</p>
			<div class="flex items-center gap-1">
				<Button
					variant="outline"
					size="xs"
					disabled={safePage <= 0}
					onclick={() => (pageIndex = Math.max(0, safePage - 1))}
				>
					Previous
				</Button>
				<Button
					variant="outline"
					size="xs"
					disabled={safePage >= pageCount - 1}
					onclick={() => (pageIndex = Math.min(pageCount - 1, safePage + 1))}
				>
					Next
				</Button>
			</div>
		</div>
	{/if}
</div>
