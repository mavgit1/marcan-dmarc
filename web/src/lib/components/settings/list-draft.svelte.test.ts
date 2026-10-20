// A list edited as text: the preview once typing pauses, the line errors it shows, and the save
// with its read-back.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LineError } from '$lib/types/LineError';
import { LIST_PREVIEW_DELAY_MS, ListDraft, type ListSpec } from './list-draft.svelte';

// A refused save carries its line errors; here as a plain field on the thrown error.
vi.mock('$lib/http', async (importOriginal) => ({
	...(await importOriginal<typeof import('$lib/http')>()),
	apiLineErrors: (err: unknown) => (err as { lines?: LineError[] }).lines ?? []
}));

type Summary = { added: string[]; errors: LineError[] };

const settle = () => vi.advanceTimersByTimeAsync(0);
const pause = () => vi.advanceTimersByTimeAsync(LIST_PREVIEW_DELAY_MS);

function spec(stored: { text: string }, over: Partial<ListSpec<Summary>> = {}): ListSpec<Summary> {
	return {
		label: 'Things',
		key: `things-${Math.random()}`,
		get: vi.fn(async () => ({ text: stored.text })),
		preview: vi.fn(async (text: string) => ({
			added: text.split('\n').filter((l) => l && !stored.text.split('\n').includes(l)),
			errors: text.includes('bad') ? [{ line: 2, message: 'Not a thing.' }] : []
		})),
		save: vi.fn(async (text: string) => {
			const before = stored.text.split('\n');
			stored.text = text;
			return { added: text.split('\n').filter((l) => l && !before.includes(l)), errors: [] };
		}),
		changes: (s) => ({ summary: s.added.length ? `${s.added.length} added` : '', details: [{ items: s.added }] }),
		...over
	};
}

async function setup(s: ListSpec<Summary>) {
	let list!: ListDraft<Summary>;
	const stop = $effect.root(() => {
		list = new ListDraft(s);
	});
	await settle();
	return { list, stop };
}

beforeEach(() => {
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
});

describe('ListDraft', () => {
	it('loads the stored text and counts its entries', async () => {
		const { list, stop } = await setup(spec({ text: 'a\n# note\n\nb' }));
		expect(list.saved).toBe('a\n# note\n\nb');
		expect(list.count).toBe(2);
		expect(list.changes).toBeNull();
		stop();
	});

	it('previews the draft once typing pauses, and only the latest draft', async () => {
		const s = spec({ text: 'a' });
		const { list, stop } = await setup(s);
		list.draft.text = 'a\nb';
		await settle();
		list.draft.text = 'a\nb\nc';
		await pause();
		expect(s.preview).toHaveBeenCalledTimes(1);
		expect(s.preview).toHaveBeenCalledWith('a\nb\nc');
		expect(list.changes).toEqual({ summary: '2 added', details: [{ items: ['b', 'c'] }] });
		// Taken back to the stored text, nothing is previewed.
		list.draft.text = 'a';
		await pause();
		expect(list.changes).toBeNull();
		stop();
	});

	it("shows the preview's line errors and holds Save", async () => {
		const { list, stop } = await setup(spec({ text: 'a' }));
		list.draft.text = 'a\nbad';
		await pause();
		expect(list.errors).toEqual([{ line: 2, message: 'Not a thing.' }]);
		expect(list.changes).toBeNull();
		expect(list.canSave).toBe(false);
		stop();
	});

	it('saves, reads the list back as stored, and says what changed', async () => {
		const stored = { text: 'a' };
		const s = spec(stored);
		const { list, stop } = await setup(s);
		list.draft.text = 'a\nb';
		const result = await list.persist();
		expect(s.save).toHaveBeenCalledWith('a\nb');
		expect(result).toEqual({ ok: true, message: 'Saved: 1 added' });
		expect(list.saved).toBe('a\nb');
		expect(list.dirty).toBe(false);
		stop();
	});

	it("keeps a refused save's line errors until the next edit", async () => {
		const lines = [{ line: 1, message: 'Unknown organization.' }];
		const s = spec(
			{ text: '' },
			{ save: vi.fn(async () => Promise.reject(Object.assign(new Error('refused'), { lines }))) }
		);
		const { list, stop } = await setup(s);
		list.draft.text = 'x';
		const result = await list.persist();
		expect(result.ok).toBe(false);
		expect(list.errors).toEqual(lines);
		expect(list.dirty).toBe(true);
		list.draft.text = 'y';
		await settle();
		expect(list.errors).toEqual([]);
		stop();
	});

	it('says so when the saved list cannot be read back', async () => {
		const stored = { text: 'a' };
		const s = spec(stored);
		const { list, stop } = await setup(s);
		vi.mocked(s.get).mockRejectedValueOnce(new Error('offline'));
		list.draft.text = 'a\nb';
		const result = await list.persist();
		expect(result.message).toBe('Saved: 1 added (reload to see the list as stored)');
		expect(list.saved).toBe('a\nb');
		stop();
	});

	it('a list without wording says Saved, and its own sameness decides what is unsaved', async () => {
		const s = spec(
			{ text: 'a\nb' },
			{ changes: undefined, same: (x, y) => x.replace(/\s/g, '') === y.replace(/\s/g, '') }
		);
		const { list, stop } = await setup(s);
		list.draft.text = 'a b';
		expect(list.dirty).toBe(false);
		list.draft.text = 'a\nc';
		expect(list.dirty).toBe(true);
		expect((await list.persist()).message).toBe('Saved');
		stop();
	});
});
