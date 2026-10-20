// The shared settings: fetched once per app session, replaced by every save's answer, and never
// overwritten by a load that started before a save.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';

vi.mock('$lib/api', () => ({ getSettings: vi.fn() }));
const getSettings = vi.mocked(api.getSettings);

function answer(folder: string): SettingsResponse {
	return settingsFixture({ mailbox: { folder }, onboarding: { state: null } });
}

/** A fresh state per test: the module holds one for the app. */
async function fresh() {
	vi.resetModules();
	return (await import('./settings.svelte')).settings;
}

/** Let settled promises run their callbacks and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

beforeEach(() => {
	getSettings.mockReset();
});

describe('settings', () => {
	it('loads once; a failure is kept as the error and retried on the next load', async () => {
		const settings = await fresh();
		getSettings.mockRejectedValueOnce(new Error('offline'));
		await expect(settings.load()).rejects.toThrow('offline');
		expect(settings.current).toBeNull();
		expect((settings.error as Error).message).toBe('offline');

		getSettings.mockResolvedValue(answer('INBOX'));
		await settings.load();
		await settings.load();
		expect(getSettings).toHaveBeenCalledTimes(2);
		expect(settings.current?.mailbox.folder).toBe('INBOX');
		expect(settings.error).toBeNull();

		await settings.reload();
		expect(getSettings).toHaveBeenCalledTimes(3);
	});

	it("a save's answer becomes current; a rejected save leaves it", async () => {
		const settings = await fresh();
		getSettings.mockResolvedValue(answer('INBOX'));
		await settings.load();
		const saved = await settings.save(Promise.resolve(answer('DMARC')));
		expect(saved.mailbox.folder).toBe('DMARC');
		expect(settings.current?.mailbox.folder).toBe('DMARC');
		await expect(settings.save(Promise.reject(new Error('400')))).rejects.toThrow('400');
		expect(settings.current?.mailbox.folder).toBe('DMARC');
		// The saved answer counts as loaded.
		await settings.load();
		expect(getSettings).toHaveBeenCalledTimes(1);
	});

	it('a load that started before a save does not overwrite it', async () => {
		const settings = await fresh();
		let finish!: (s: SettingsResponse) => void;
		getSettings.mockReturnValueOnce(new Promise((done) => (finish = done)));
		const loading = settings.load();
		await settings.save(Promise.resolve(answer('DMARC')));
		finish(answer('INBOX'));
		await loading;
		expect(settings.current?.mailbox.folder).toBe('DMARC');
	});

	it('reloads on every refresh tick once loaded, never before', async () => {
		const settings = await fresh();
		// The module instance the fresh state reads.
		const { refresh } = await import('./refresh.svelte');
		refresh.refresh();
		await settle();
		expect(getSettings).not.toHaveBeenCalled();

		getSettings.mockResolvedValue(answer('INBOX'));
		await settings.load();
		await settle();
		expect(getSettings).toHaveBeenCalledTimes(1);

		getSettings.mockResolvedValue(answer('Changed elsewhere'));
		refresh.refresh();
		await settle();
		expect(getSettings).toHaveBeenCalledTimes(2);
		expect(settings.current?.mailbox.folder).toBe('Changed elsewhere');

		// A failed refresh keeps what was loaded.
		getSettings.mockRejectedValueOnce(new Error('offline'));
		refresh.refresh();
		await settle();
		expect(getSettings).toHaveBeenCalledTimes(3);
		expect(settings.current?.mailbox.folder).toBe('Changed elsewhere');
		expect((settings.error as Error).message).toBe('offline');
	});
});
