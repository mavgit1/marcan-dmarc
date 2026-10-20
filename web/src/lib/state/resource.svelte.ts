import { untrack } from 'svelte';
import { refresh } from './refresh.svelte';

export type ResourceOptions<T, A = never> = {
	/**
	 * What counts as the same content. Data loaded for this key stays on screen while the
	 * next load for it is pending, and after it fails; another key starts empty.
	 */
	key: () => string;
	/**
	 * Fetch the data. It runs untracked: nothing it reads reloads it on its own. What should
	 * trigger a load goes in `key` (other content) or `reloadOn` (the same content again), and
	 * `refresh.tick` (unless `refetchOnRefresh` is false) and `reload()` load as well. `arg` is
	 * what `reload(arg)` was called with, for that one load only. Return `null` when there is
	 * nothing to load (e.g. an invalid id the page reports itself, or a check that is not
	 * wanted yet).
	 */
	load: (arg?: A) => Promise<T> | null;
	/**
	 * Read here the reactive values that ask for the current key's content again without
	 * changing it (a sort, a page, the time range of a page keyed on its entity, an import
	 * tick): the shown data stays until the answer lands.
	 */
	reloadOn?: () => unknown;
	/**
	 * Reload on every `refresh.tick` (the timer, import ticks). Default
	 * true; false for a load with its own freshness, e.g. a live DNS check.
	 */
	refetchOnRefresh?: boolean;
	/**
	 * An answer already in hand for one key (fetched by another component): shown at once,
	 * and not loaded again until the key changes or `reload()` is called.
	 */
	seed?: { key: string; data: T } | null;
	/** Call `refresh.markUpdated()` after each successful load (the header's "Updated"). */
	markUpdated?: boolean;
	/** After each successful load; `previous` is the data it replaces for the same key. */
	onData?: (data: T, previous: T | null) => void;
};

/**
 * One keyed API load of a component, refetched on every `refresh.tick`. A refresh never
 * blanks the page: `data` keeps the last good answer for the current key, `error` is set
 * when the latest attempt for it failed (shown inline next to that data, or as the page's
 * error when nothing was ever loaded), and a response that is no longer the latest request
 * is dropped. Loaders are untracked: put everything that should trigger a reload in `key`
 * or `reloadOn`. Create it during component initialisation (it owns an `$effect`).
 */
export class Resource<T, A = never> {
	#last = $state.raw<{ key: string; data: T } | null>(null);
	#failed = $state.raw<{ key: string; error: unknown } | null>(null);
	#pending = $state<string | null>(null);
	#reloads = $state(0);
	/** What the pending `reload(arg)` was called with; consumed by the load it triggers. */
	#reloadArg: A | undefined = undefined;
	#keyFn: () => string;
	#key = $derived.by(() => this.#keyFn());

	constructor(options: ResourceOptions<T, A>) {
		this.#keyFn = options.key;
		const seed = options.seed ?? null;
		if (seed) this.#last = seed;
		let request = 0;
		let seeded = seed !== null;
		$effect(() => {
			if (options.refetchOnRefresh !== false) void refresh.tick;
			void this.#reloads;
			const key = options.key();
			options.reloadOn?.();
			const arg = this.#reloadArg;
			this.#reloadArg = undefined;
			if (seeded) {
				// The seed answers the first run for its key; anything else loads.
				seeded = false;
				if (key === seed?.key) return;
			}
			let promise: Promise<T> | null;
			try {
				promise = untrack(() => options.load(arg));
			} catch (error) {
				promise = Promise.reject(error);
			}
			const id = ++request;
			if (promise === null) {
				this.#pending = null;
				return;
			}
			this.#pending = key;
			promise.then(
				(data) => {
					if (id !== request) return;
					const previous = untrack(() => (this.#last?.key === key ? this.#last.data : null));
					this.#last = { key, data };
					this.#failed = null;
					this.#pending = null;
					if (options.markUpdated) refresh.markUpdated();
					options.onData?.(data, previous);
				},
				(error: unknown) => {
					if (id !== request) return;
					this.#failed = { key, error };
					this.#pending = null;
				}
			);
			return () => {
				// Superseded or unmounted: its answer must not land.
				request += 1;
			};
		});
	}

	/** The last good data for the current key, or `null` before the first answer. */
	get data(): T | null {
		const last = this.#last;
		return last !== null && last.key === this.#key ? last.data : null;
	}

	/** What the latest load for the current key threw, until a load succeeds. */
	get error(): unknown {
		const failed = this.#failed;
		return failed !== null && failed.key === this.#key ? failed.error : null;
	}

	/** A load for the current key is in flight. */
	get loading(): boolean {
		return this.#pending !== null && this.#pending === this.#key;
	}

	/**
	 * Load again now (a Re-check button); the current data stays until the answer lands.
	 * `arg` reaches that one load (e.g. `{ recheck: true }` to bypass a server cache).
	 */
	reload(arg?: A): void {
		this.#reloadArg = arg;
		this.#reloads += 1;
	}

	/**
	 * Replace the current key's data with what a save returned, so the screen agrees with the
	 * server before the next load. No-op before the first answer.
	 */
	update(fn: (data: T) => T): void {
		const last = untrack(() => this.#last);
		const key = untrack(() => this.#key);
		if (last === null || last.key !== key) return;
		this.#last = { key, data: fn(last.data) };
	}
}

/** `new Resource(options)`; call it during component initialisation. */
export function resource<T, A = never>(options: ResourceOptions<T, A>): Resource<T, A> {
	return new Resource(options);
}
