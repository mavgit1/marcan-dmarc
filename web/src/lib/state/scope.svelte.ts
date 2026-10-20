import { untrack } from 'svelte';
import { getScope } from '$lib/api';
import type { ScopeOptions } from '$lib/types/ScopeOptions';
import { refresh } from './refresh.svelte';

/**
 * App-wide `GET /api/scope`: organizations (default first) and monitored domains, ids and
 * names only. The one source for every picker that needs names (header scope filter, export
 * dialog, add domains, move and assign pickers), fetched at most once per `refresh.tick`:
 * every change to organizations or domains ends in `refresh.refresh()`.
 */
class ScopeState {
	/** The last answer; kept while a newer one loads or after one fails. */
	current = $state.raw<ScopeOptions | null>(null);
	#promise: Promise<ScopeOptions> | null = null;
	#tick = -1;

	/** The scope of the current refresh tick, fetched now unless already fetched or in flight. */
	load(): Promise<ScopeOptions> {
		const tick = untrack(() => refresh.tick);
		if (this.#promise && this.#tick === tick) return this.#promise;
		this.#tick = tick;
		const promise = getScope().then(
			(scope) => {
				if (this.#promise === promise) this.current = scope;
				return scope;
			},
			(error: unknown) => {
				// The next call tries again.
				if (this.#promise === promise) this.#promise = null;
				throw error;
			}
		);
		this.#promise = promise;
		return promise;
	}
}

export const scopeOptions = new ScopeState();
