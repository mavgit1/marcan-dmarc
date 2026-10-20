import type { Snippet } from 'svelte';

type ColumnAlign = 'left' | 'right' | 'center';

export type ColumnDef<T> = {
	id: string;
	header: string;
	/** Extract the sort / default-display value. */
	accessor: (row: T) => string | number | null | undefined;
	/** Format the accessor value when no `cell` snippet is provided. */
	format?: (value: string | number | null | undefined, row: T) => string;
	/** Client-side sort by `accessor` (ignored when the table is server-driven). */
	sortable?: boolean;
	/** API sort key (`sort=key` / `-key`) when the table is server-driven. */
	sortKey?: string;
	align?: ColumnAlign;
	/** Right-align and use tabular numerals. */
	numeric?: boolean;
	class?: string;
	headerClass?: string;
};

export function defaultCellText<T>(column: ColumnDef<T>, row: T): string {
	const value = column.accessor(row);
	if (column.format) return column.format(value, row);
	if (value == null || value === '') return '—';
	return String(value);
}

export function columnAlign<T>(column: ColumnDef<T>): ColumnAlign {
	if (column.align) return column.align;
	return column.numeric ? 'right' : 'left';
}

type DataTableCellContext<T> = {
	column: ColumnDef<T>;
	row: T;
	value: string | number | null | undefined;
};

export type DataTableCellSnippet<T> = Snippet<[DataTableCellContext<T>]>;

/**
 * A server-paginated list: the rows are one page, so sorting or paging them locally would
 * misrepresent the list. The table shows `sort` and asks for changes instead.
 */
export type ServerTable = {
	/** Current API sort: `key` ascending, `-key` descending. */
	sort: string;
	onSort: (sort: string) => void;
	total: number;
	limit: number;
	offset: number;
	onPage: (offset: number) => void;
};

/** Split an API sort (`-key`) into key and direction. */
export function parseSort(sort: string): { key: string; dir: 'asc' | 'desc' } {
	return sort.startsWith('-') ? { key: sort.slice(1), dir: 'desc' } : { key: sort, dir: 'asc' };
}

/**
 * The next API sort after clicking a column: flips the current column, else starts
 * numbers high-to-low and text A-to-Z.
 */
export function nextSort(current: string, key: string, numeric: boolean): string {
	const { key: currentKey, dir } = parseSort(current);
	if (currentKey === key) return dir === 'desc' ? key : `-${key}`;
	return numeric ? `-${key}` : key;
}
