import type { ServerTable } from '$lib/components/data-table';

/**
 * Sort and page of one server-paginated list. The page resets to the first whenever
 * the list's scope (filters) or sort changes, so a stale offset never lands past the end.
 */
export class ServerList {
	sort = $state('');
	#offset = $state(0);
	#offsetKey = $state('');

	constructor(defaultSort: string) {
		this.sort = defaultSort;
	}

	/** Offset to request for `scopeKey` (the current filters). */
	offsetFor(scopeKey: string): number {
		return scopeKey === this.#offsetKey ? this.#offset : 0;
	}

	/** Props for `DataTable`'s `server` mode, for a loaded page. */
	table(scopeKey: string, page: { total: number; limit: number; offset: number }): ServerTable {
		return {
			sort: this.sort,
			onSort: (sort) => {
				this.sort = sort;
				this.#offset = 0;
			},
			total: page.total,
			limit: page.limit,
			offset: page.offset,
			onPage: (offset) => {
				this.#offsetKey = scopeKey;
				this.#offset = offset;
			}
		};
	}
}
