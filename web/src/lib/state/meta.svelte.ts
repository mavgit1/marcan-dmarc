import { getMeta } from '$lib/api';
import type { Meta } from '$lib/types/Meta';

/**
 * `GET /api/meta`: the rules' fixed numbers (99 / 95, stop days, key sizes, score points)
 * for help text, and what the app frame shows on every page: the source URL, the free-software
 * notice and whether it is shown, and the operator's branding. Fetched once; a failed fetch is
 * retried on the next `load()`, and the license and branding saves `reload()` it. Text quoting
 * it reads "…" until loaded.
 */
class MetaState {
	current = $state.raw<Meta | null>(null);
	#promise: Promise<Meta> | null = null;

	load(): Promise<Meta> {
		if (this.#promise) return this.#promise;
		const promise = getMeta().then(
			(meta) => {
				this.current = meta;
				return meta;
			},
			(error: unknown) => {
				this.#promise = null;
				throw error;
			}
		);
		this.#promise = promise;
		return promise;
	}

	/** Fetch again (after a license or branding save); the last answer stays until then. */
	reload(): Promise<Meta> {
		this.#promise = null;
		return this.load();
	}
}

export const meta = new MetaState();
