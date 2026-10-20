// The keyed loader every page goes through: a late answer never lands, a failed refresh keeps
// the last data, `reload(arg)` asks again (handing `arg` to the loader), `reloadOn` asks again for
// the same key, a seed answers the first run, and nothing the loader reads reloads it.
import { describe, expect, it } from 'vitest';
import { refresh } from './refresh.svelte';
import { resource, type Resource } from './resource.svelte';

/** A promise settled by hand, so answers can land in any order. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

/** Let settled promises run their callbacks and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

/** Builds the resource inside an effect root and hands back the controls once its effect ran. */
async function setup(key: () => string, load: () => Promise<string> | null, extra: object = {}) {
	let r!: Resource<string>;
	const stop = $effect.root(() => {
		r = resource({ key, load, ...extra });
	});
	await settle();
	return { r, stop };
}

describe('resource', () => {
	it('starts empty and loading, then holds the answer', async () => {
		const d = deferred<string>();
		const { r, stop } = await setup(
			() => 'a',
			() => d.promise
		);
		expect(r.data).toBeNull();
		expect(r.loading).toBe(true);
		d.resolve('first');
		await settle();
		expect(r.data).toBe('first');
		expect(r.loading).toBe(false);
		expect(r.error).toBeNull();
		stop();
	});

	it('drops an answer that is no longer the latest request', async () => {
		const answers: ReturnType<typeof deferred<string>>[] = [];
		const { r, stop } = await setup(
			() => 'a',
			() => {
				const d = deferred<string>();
				answers.push(d);
				return d.promise;
			}
		);
		refresh.refresh();
		await settle();
		expect(answers).toHaveLength(2);
		answers[1].resolve('second');
		await settle();
		answers[0].resolve('first, late');
		await settle();
		expect(r.data).toBe('second');
		stop();
	});

	it('keeps the last data when a refresh fails, and reports the error beside it', async () => {
		const answers: ReturnType<typeof deferred<string>>[] = [];
		const { r, stop } = await setup(
			() => 'a',
			() => {
				const d = deferred<string>();
				answers.push(d);
				return d.promise;
			}
		);
		answers[0].resolve('good');
		await settle();
		refresh.refresh();
		await settle();
		expect(r.loading).toBe(true);
		expect(r.data).toBe('good');
		answers[1].reject(new Error('down'));
		await settle();
		expect(r.data).toBe('good');
		expect((r.error as Error).message).toBe('down');
		expect(r.loading).toBe(false);
		stop();
	});

	it('shows another key empty, and the old key its data again', async () => {
		let key = $state('a');
		const answers: ReturnType<typeof deferred<string>>[] = [];
		const { r, stop } = await setup(
			() => key,
			() => {
				const d = deferred<string>();
				answers.push(d);
				return d.promise;
			}
		);
		answers[0].resolve('data for a');
		await settle();
		expect(r.data).toBe('data for a');
		key = 'b';
		await settle();
		expect(r.data).toBeNull();
		expect(r.loading).toBe(true);
		answers[1].resolve('data for b');
		await settle();
		expect(r.data).toBe('data for b');
		stop();
	});

	it('reload() loads again on demand; refetchOnRefresh false ignores the app refresh', async () => {
		let calls = 0;
		const { r, stop } = await setup(
			() => 'a',
			() => Promise.resolve(`call ${++calls}`),
			{ refetchOnRefresh: false }
		);
		expect(r.data).toBe('call 1');
		refresh.refresh();
		await settle();
		expect(calls).toBe(1);
		r.reload();
		await settle();
		expect(r.data).toBe('call 2');
		stop();
	});

	it('a seed answers the first run without loading; update() replaces the data locally', async () => {
		let calls = 0;
		const { r, stop } = await setup(
			() => 'a',
			() => Promise.resolve(`call ${++calls}`),
			{ seed: { key: 'a', data: 'seeded' } }
		);
		expect(r.data).toBe('seeded');
		expect(calls).toBe(0);
		r.update((d) => `${d}!`);
		expect(r.data).toBe('seeded!');
		r.reload();
		await settle();
		expect(r.data).toBe('call 1');
		stop();
	});

	it('the loader is untracked: a signal it reads does not reload it, one in reloadOn does', async () => {
		let polled = $state(0);
		let sort = $state('asc');
		let calls = 0;
		const { r, stop } = await setup(
			() => 'a',
			() => Promise.resolve(`call ${++calls} ${polled} ${sort}`),
			{ reloadOn: () => sort }
		);
		expect(r.data).toBe('call 1 0 asc');
		polled += 1;
		await settle();
		expect(calls).toBe(1);
		sort = 'desc';
		await settle();
		expect(calls).toBe(2);
		expect(r.data).toBe('call 2 1 desc');
		stop();
	});

	it('reloadOn keeps the shown data while the same key loads again', async () => {
		let sort = $state('asc');
		const answers: ReturnType<typeof deferred<string>>[] = [];
		const { r, stop } = await setup(
			() => 'a',
			() => {
				const d = deferred<string>();
				answers.push(d);
				return d.promise;
			},
			{ reloadOn: () => sort }
		);
		answers[0].resolve('asc rows');
		await settle();
		sort = 'desc';
		await settle();
		expect(r.loading).toBe(true);
		expect(r.data).toBe('asc rows');
		answers[1].resolve('desc rows');
		await settle();
		expect(r.data).toBe('desc rows');
		stop();
	});

	it('reload(arg) hands its argument to that one load; the next load gets none', async () => {
		const seen: unknown[] = [];
		let r!: Resource<string, { recheck?: boolean }>;
		const stop = $effect.root(() => {
			r = resource({
				key: () => 'a',
				load: (opts?: { recheck?: boolean }) => {
					seen.push(opts);
					return Promise.resolve(`recheck ${opts?.recheck ?? false}`);
				}
			});
		});
		await settle();
		expect(r.data).toBe('recheck false');
		r.reload({ recheck: true });
		await settle();
		expect(r.data).toBe('recheck true');
		r.reload();
		await settle();
		refresh.refresh();
		await settle();
		expect(seen).toEqual([undefined, { recheck: true }, undefined, undefined]);
		stop();
	});

	it('load() returning null means nothing to load', async () => {
		const { r, stop } = await setup(
			() => '',
			() => null
		);
		expect(r.data).toBeNull();
		expect(r.loading).toBe(false);
		expect(r.error).toBeNull();
		stop();
	});
});
