import { untrack } from 'svelte';
import { getSettings } from '$lib/api';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { refresh } from './refresh.svelte';

/**
 * App-wide `GET /api/settings`: every setting by group, loaded once per app session and again
 * on every later `refresh.tick` (the auto-refresh, a finished import), so
 * a change made elsewhere shows up. Every Settings section (a `SettingsForm` with
 * `{ shared: settings }`), the setup, the guide, the domain overview and the dashboard
 * read `current`; every save goes through `save()`, so they all see what the server stored. A
 * failed load is retried on the next `load()`.
 */
class SettingsState {
	/** `null` until the first answer. */
	current = $state.raw<SettingsResponse | null>(null);
	/** Why the last load failed; cleared by the next answer. */
	error = $state.raw<unknown>(null);
	#promise: Promise<SettingsResponse> | null = null;
	/** Bumped by every answer, so a load that started before a save cannot overwrite it. */
	#version = 0;
	#watching = false;

	/** The settings, fetched at most once (until a failure, `reload()` or a refresh). */
	load(): Promise<SettingsResponse> {
		this.#reloadOnRefresh();
		if (this.#promise) return this.#promise;
		const version = this.#version;
		const promise = getSettings().then(
			(next) => {
				if (version === this.#version) this.#set(next);
				return this.current ?? next;
			},
			(error: unknown) => {
				if (this.#promise === promise) this.#promise = null;
				if (version === this.#version) this.error = error;
				throw error;
			}
		);
		this.#promise = promise;
		return promise;
	}

	/** Fetch again (a retry, or a change made outside this app session). */
	reload(): Promise<SettingsResponse> {
		this.#promise = null;
		return this.load();
	}

	/**
	 * One save (a `save*` request of `$lib/api`, which answers every setting): its answer becomes
	 * `current`. Returns it; a rejected save leaves `current` as it was.
	 */
	async save(request: Promise<SettingsResponse>): Promise<SettingsResponse> {
		const next = await request;
		this.#set(next);
		this.#promise = Promise.resolve(next);
		return next;
	}

	/** From the first `load()` on, every new `refresh.tick` reloads (a failure stays in `error`). */
	#reloadOnRefresh() {
		if (this.#watching) return;
		this.#watching = true;
		let seen = untrack(() => refresh.tick);
		$effect.root(() => {
			$effect(() => {
				const tick = refresh.tick;
				if (tick === seen) return;
				seen = tick;
				untrack(() => this.reload()).catch(() => {});
			});
		});
	}

	#set(next: SettingsResponse) {
		this.#version += 1;
		this.current = next;
		this.error = null;
	}
}

export const settings = new SettingsState();
