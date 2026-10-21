// The version check's switch: on by default, saved as the whole group; a save reloads meta, which
// carries the newer version the server found.
import { describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import * as api from '$lib/api';
import { UpdatesModel } from './updates-draft.svelte';

vi.mock('$lib/api', () => ({ getSettings: vi.fn(), getMeta: vi.fn(), saveUpdates: vi.fn() }));

const settle = () => new Promise<void>((done) => setTimeout(done));

describe('version check', () => {
	it('starts on by default, with nothing found yet', async () => {
		let m!: UpdatesModel;
		const stop = $effect.root(() => {
			m = new UpdatesModel({ source: { value: () => settingsFixture() } });
		});
		await settle();
		expect(m.draft).toEqual({ check: true });
		stop();
	});

	it('saves the switch and reloads meta', async () => {
		const off = settingsFixture({ updates: { check: false } });
		const on = settingsFixture({ updates: { check: true } });
		vi.mocked(api.saveUpdates).mockResolvedValue(on);
		vi.mocked(api.getMeta).mockResolvedValue({} as Awaited<ReturnType<typeof api.getMeta>>);
		let m!: UpdatesModel;
		const stop = $effect.root(() => {
			m = new UpdatesModel({ source: { value: () => off } });
		});
		await settle();
		expect(m.draft).toEqual({ check: false });
		m.draft.check = true;
		expect(m.dirty).toBe(true);
		await m.persist();
		expect(api.saveUpdates).toHaveBeenCalledWith({ check: true });
		expect(m.dirty).toBe(false);
		expect(api.getMeta).toHaveBeenCalled();
		stop();
	});
});
