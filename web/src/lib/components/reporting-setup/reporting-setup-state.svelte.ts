import { getReportingSetup } from '$lib/api';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import { resource, type Resource } from '$lib/state/resource.svelte';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

/**
 * The reporting setup as the server decided it (`GET /api/reporting-setup`): every domain's
 * status, or one domain's (`domainId`). Re-check asks again bypassing the server's DNS caches,
 * for the whole list or one row; a row whose reports arrive is looked up only when its record
 * is shown. Create it during component initialisation (it owns a `resource`).
 */
export class ReportingSetupState {
	readonly source: Resource<ReportingSetupResponse, { recheck?: boolean }>;
	/** Rows being looked up on their own. */
	readonly busy = new SvelteSet<number>();
	/** What the last lookup of a row on its own threw. */
	readonly failed = new SvelteMap<number, unknown>();

	constructor(
		options: {
			/** One domain only. */
			domainId?: () => number | null;
			/** `false` while there is nothing to ask. */
			enabled?: () => boolean;
			/** Ask again without changing what is shown (an import adding domains and reports). */
			reloadOn?: () => unknown;
		} = {}
	) {
		const domainId = () => options.domainId?.() ?? null;
		const enabled = () => options.enabled?.() ?? true;
		this.source = resource({
			key: () => `reporting-setup|${domainId() ?? 'all'}|${enabled()}`,
			load: (arg) =>
				enabled() ? getReportingSetup({ domain_id: domainId() ?? undefined, refresh: arg?.recheck }) : null,
			reloadOn: options.reloadOn
		});
	}

	get data(): ReportingSetupResponse | null {
		return this.source.data;
	}

	/** Re-check everything, bypassing the server's DNS caches. */
	recheck(): void {
		this.source.reload({ recheck: true });
	}

	/**
	 * Look up one row on its own: Re-check (`refresh`), or the record of a row whose reports
	 * arrive. It keeps its place in the list until the next load sorts it.
	 */
	async lookup(id: number, refresh: boolean): Promise<void> {
		if (this.busy.has(id)) return;
		this.busy.add(id);
		this.failed.delete(id);
		try {
			const row = (await getReportingSetup({ domain_id: id, refresh })).domains[0];
			if (row) this.source.update((data) => ({ ...data, domains: data.domains.map((d) => (d.id === id ? row : d)) }));
		} catch (err) {
			this.failed.set(id, err);
		} finally {
			this.busy.delete(id);
		}
	}
}
