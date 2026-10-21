// The operator license section's draft: the key is never shown back, a typed key replaces the
// stored one, Remove clears it, and every save reloads `meta`. There is no notice switch.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import type { LicenseSettingsResponse } from '$lib/types/LicenseSettingsResponse';
import type { Meta } from '$lib/types/Meta';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import { BLANK, LICENSE_STATE, LicenseModel, toRequest, type LicenseDraft } from './license-draft.svelte';

vi.mock('$lib/api', () => ({
	getSettings: vi.fn(),
	getMeta: vi.fn(),
	saveLicense: vi.fn()
}));

const saveLicense = vi.mocked(api.saveLicense);
const getMeta = vi.mocked(api.getMeta);

const settle = () => new Promise<void>((done) => setTimeout(done));

const NONE: LicenseSettingsResponse = { has_key: false, state: 'no_key', licensed_to: null, key_hint: null };
const LICENSED: LicenseSettingsResponse = {
	has_key: true,
	state: 'active',
	licensed_to: 'you@company.example',
	key_hint: 'you@company.example · …NJ69Cw'
};

function server(license: LicenseSettingsResponse = NONE): SettingsResponse {
	return settingsFixture({ license });
}

async function setup(saved: SettingsResponse | null) {
	let m!: LicenseModel;
	const stop = $effect.root(() => {
		m = new LicenseModel({ source: { value: () => saved } });
	});
	await settle();
	return { m, stop };
}

const draft = (over: Partial<LicenseDraft> = {}): LicenseDraft => ({ ...BLANK, ...over });

beforeEach(() => {
	vi.clearAllMocks();
	getMeta.mockResolvedValue({} as Meta);
});

describe('the draft and the request', () => {
	it('never puts the stored key in the draft', async () => {
		const { m, stop } = await setup(server(LICENSED));
		expect(m.draft).toEqual(BLANK);
		expect(BLANK).toEqual({ key: '', remove: false });
		stop();
	});

	it('a typed key or a removal is a change; blanks are not', async () => {
		const { m, stop } = await setup(server(LICENSED));
		m.draft.key = '   ';
		expect(m.dirty).toBe(false);
		m.draft.key = 'you@company.example.AbC';
		expect(m.dirty).toBe(true);
		m.discard();
		m.removeKey();
		expect(m.dirty).toBe(true);
		stop();
	});

	it('keeps, replaces or removes the key', () => {
		expect(toRequest(draft())).toEqual({});
		expect(toRequest(draft({ key: ' you@company.example.AbC ' }))).toEqual({ operator_key: 'you@company.example.AbC' });
		expect(toRequest(draft({ remove: true }))).toEqual({ operator_key: '' });
	});
});

describe('LICENSE_STATE', () => {
	it('colours and words each state the server decides', () => {
		expect(LICENSE_STATE.active).toEqual({
			tone: 'pass',
			title: 'Operator license active.',
			detail: 'Client emails carry your name, not the free-software notice.'
		});
		expect(LICENSE_STATE.no_key).toEqual({
			tone: 'neutral',
			title: 'No operator license.',
			detail: 'Client emails carry the free-software notice.'
		});
		expect(LICENSE_STATE.invalid).toEqual({
			tone: 'fail',
			title: 'The stored operator key is not valid.',
			detail: null
		});
	});
});

describe('LicenseModel', () => {
	it('saves a typed key, says who it is licensed to and reloads meta', async () => {
		saveLicense.mockResolvedValue(server(LICENSED));
		const { m, stop } = await setup(server());
		expect(m.licensedTo).toBeNull();
		m.draft.key = ' you@company.example.AbC ';
		await settle();
		m.submit();
		await settle();
		expect(saveLicense).toHaveBeenCalledWith({ operator_key: 'you@company.example.AbC' });
		expect(m.save.feedback).toEqual({ ok: true, message: 'Saved: licensed to you@company.example' });
		expect(getMeta).toHaveBeenCalled();
		expect(m.draft.key).toBe('');
		expect(m.licensedTo).toBe('you@company.example');
		// The locked field names the stored key by the server's hint.
		expect(m.keyHint).toBe('you@company.example · …NJ69Cw');
		stop();
	});

	it('shows the server refusal of an invalid key, and keeps it typed to correct', async () => {
		saveLicense.mockRejectedValue(new Error('This key is not valid.'));
		const { m, stop } = await setup(server());
		m.draft.key = 'nonsense';
		await settle();
		m.submit();
		await settle();
		expect(m.save.feedback).toEqual({ ok: false, message: 'This key is not valid.' });
		expect(m.draft.key).toBe('nonsense');
		expect(m.dirty).toBe(true);
		stop();
	});

	it('keeps no copy of a saved key: the field is empty and nothing it reads holds the key', async () => {
		const key = 'you@company.example.SecretKeyAbC';
		saveLicense.mockResolvedValue(server(LICENSED));
		const { m, stop } = await setup(server());
		m.draft.key = key;
		await settle();
		m.submit();
		await settle();
		expect(m.draft).toEqual({ key: '', remove: false });
		expect(m.dirty).toBe(false);
		for (const held of [m.draft, m.base, m.saved]) expect(JSON.stringify(held)).not.toContain('SecretKey');
		expect(m.licensedTo).toBe('you@company.example');
		stop();
	});

	it('removes the key on save', async () => {
		saveLicense.mockResolvedValue(server());
		const { m, stop } = await setup(server(LICENSED));
		m.removeKey();
		await settle();
		expect(m.dirty).toBe(true);
		m.submit();
		await settle();
		expect(saveLicense).toHaveBeenCalledWith({ operator_key: '' });
		expect(m.save.feedback).toEqual({ ok: true, message: 'Key removed' });
		stop();
	});
});
