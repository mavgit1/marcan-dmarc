// The branding section's draft: the operator name and the logo change (a picked image, or
// "Remove logo"), the client-side checks worded like the server's, and the save (the name, then
// the upload or removal, then the stored logo shown is asked for again).
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import type { Meta } from '$lib/types/Meta';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import { storedLogo } from '$lib/state/logo.svelte';
import { meta } from '$lib/state/meta.svelte';
import type { EmailPreview } from '$lib/types/EmailPreview';
import {
	BrandingModel,
	PREVIEW_DELAY_MS,
	kilobytes,
	previewNote,
	previewRequest,
	problemWith,
	type BrandingDraft
} from './branding-draft.svelte';

/** No name, the logo as it is. */
const KEEP: BrandingDraft = { operator_name: '', logo: { kind: 'keep' } };

vi.mock('$lib/api', () => ({
	getSettings: vi.fn(),
	getMeta: vi.fn(),
	saveBranding: vi.fn(),
	uploadLogo: vi.fn(),
	deleteLogo: vi.fn(),
	previewBranding: vi.fn(),
	logoUrl: (revision: number) => `/api/settings/branding/logo?r=${revision}`
}));

const saveBranding = vi.mocked(api.saveBranding);
const uploadLogo = vi.mocked(api.uploadLogo);
const deleteLogo = vi.mocked(api.deleteLogo);
const getMeta = vi.mocked(api.getMeta);
const previewBranding = vi.mocked(api.previewBranding);

const NOTICE = 'Sent by Acme IT with Marcan, free software …';
const TITLE = 'DMARC report for Example Ltd, September 2026';
const PREVIEW: EmailPreview = {
	operator_name: 'Acme IT',
	notice_shown: true,
	current: {
		notice_shown: true,
		from_name: 'Acme IT via Marcan',
		top: {
			software: { mark: { width: 20, height: 20 }, wordmark: 'Marcan', rest: ', free software' },
			operator: { prominent: false, logo: null, name: 'Managed by Acme IT' }
		},
		title: TITLE,
		sign_off: NOTICE
	},
	other: {
		notice_shown: false,
		from_name: 'Acme IT',
		top: { software: null, operator: { prominent: true, logo: null, name: 'Acme IT' } },
		title: TITLE,
		sign_off: 'Sent by Acme IT.'
	}
};
/** Long enough for the preview to be asked again after the draft changed. */
const pause = () => new Promise<void>((done) => setTimeout(done, PREVIEW_DELAY_MS + 50));

const settle = () => new Promise<void>((done) => setTimeout(done));

const MAX = 200 * 1024;
const png = (size = 100, type = 'image/png') => new File([new Uint8Array(size)], 'logo.png', { type });

function server(operator_name = '', has_logo = false): SettingsResponse {
	return settingsFixture({
		branding: {
			operator_name,
			has_logo,
			logo_type: has_logo ? 'png' : null,
			logo_bytes: has_logo ? 12_345 : null
		}
	});
}

async function setup(saved: SettingsResponse | null) {
	let m!: BrandingModel;
	const stop = $effect.root(() => {
		m = new BrandingModel({ source: { value: () => saved } });
	});
	await settle();
	return { m, stop };
}

beforeEach(async () => {
	vi.clearAllMocks();
	getMeta.mockResolvedValue({ logo_max_bytes: MAX } as Meta);
	previewBranding.mockResolvedValue(PREVIEW);
	await meta.reload();
});

describe('the draft and what is wrong with it', () => {
	it('is the saved group; the logo starts unchanged', async () => {
		const { m, stop } = await setup(server('Acme IT', true));
		expect(m.draft).toEqual({ operator_name: 'Acme IT', logo: { kind: 'keep' } });
		stop();
	});

	it('is clean for the saved name written with spaces, dirty for any logo change', async () => {
		const { m, stop } = await setup(server('Acme IT'));
		const change = (over: Partial<typeof m.draft>) => {
			m.discard();
			Object.assign(m.draft, over);
			return m.dirty;
		};
		expect(change({ operator_name: ' Acme IT ' })).toBe(false);
		expect(change({ operator_name: 'Acme' })).toBe(true);
		expect(change({ logo: { kind: 'remove' } })).toBe(true);
		expect(change({ logo: { kind: 'replace', file: png(), size: null } })).toBe(true);
		stop();
	});

	it('names a picked file of the wrong type or size, like the server', () => {
		const pick = (file: File) => problemWith({ ...KEEP, logo: { kind: 'replace', file, size: null } }, MAX);
		expect(problemWith(KEEP, MAX)).toBeNull();
		expect(pick(png())).toBeNull();
		expect(pick(png(100, 'image/webp'))).toBeNull();
		expect(pick(png(MAX))).toBeNull();
		expect(pick(png(100, 'image/svg+xml'))).toBe('The logo must be a PNG, JPEG or WebP image.');
		expect(pick(png(100, 'image/gif'))).toBe('The logo must be a PNG, JPEG or WebP image.');
		expect(pick(png(0))).toBe('The logo file is empty.');
		expect(pick(png(300 * 1024))).toBe(
			'The logo is 300 KB; it can be at most 200 KB. Save it smaller or as a JPEG or WebP.'
		);
		// Without the server's limit, only the type is checked here.
		expect(problemWith({ ...KEEP, logo: { kind: 'replace', file: png(300 * 1024), size: null } }, null)).toBeNull();
	});

	it('words sizes in whole kilobytes, rounded up', () => {
		expect(kilobytes(1)).toBe('1 KB');
		expect(kilobytes(1024)).toBe('1 KB');
		expect(kilobytes(1025)).toBe('2 KB');
		expect(kilobytes(MAX)).toBe('200 KB');
	});
});

describe('previewRequest / previewNote', () => {
	it('asks with the typed name and whether a logo would top the email', () => {
		const saved = server('Acme IT', true);
		expect(previewRequest({ operator_name: ' Acme ', logo: { kind: 'keep' } }, saved)).toEqual({
			operator_name: 'Acme',
			has_logo: true
		});
		expect(previewRequest({ operator_name: '', logo: { kind: 'remove' } }, saved)).toEqual({
			operator_name: '',
			has_logo: false
		});
		// A picked logo goes with its size once the browser has read it; until then nothing is asked.
		const size = { width: 1200, height: 300 };
		expect(previewRequest({ operator_name: '', logo: { kind: 'replace', file: png(), size } }, server())).toEqual({
			operator_name: '',
			has_logo: true,
			logo_size: size
		});
		expect(
			previewRequest({ operator_name: '', logo: { kind: 'replace', file: png(), size: null } }, server())
		).toBeNull();
		expect(previewRequest(KEEP, server())?.has_logo).toBe(false);
	});

	it('picks the line under the preview from the server state', () => {
		expect(previewNote(PREVIEW)).toBe('notice');
		expect(previewNote({ ...PREVIEW, notice_shown: false, current: PREVIEW.other, other: PREVIEW.current })).toBe(
			'licensed'
		);
		expect(previewNote({ ...PREVIEW, operator_name: null })).toBe('no_name');
	});
});

describe('BrandingModel', () => {
	it('previews the draft, not only the saved name, once typing pauses', async () => {
		const { m, stop } = await setup(server('Acme IT'));
		await pause();
		expect(previewBranding).toHaveBeenLastCalledWith({ operator_name: 'Acme IT', has_logo: false });
		expect(m.preview).toEqual(PREVIEW);
		const typed: EmailPreview = {
			...PREVIEW,
			operator_name: 'Acme',
			current: { ...PREVIEW.current, sign_off: 'Sent by Acme with Marcan, …' }
		};
		previewBranding.mockResolvedValue(typed);
		m.draft.operator_name = 'Acm';
		await settle();
		m.draft.operator_name = 'Acme';
		await pause();
		// One request for the pause, with the last draft.
		expect(previewBranding).toHaveBeenCalledTimes(2);
		expect(previewBranding).toHaveBeenLastCalledWith({ operator_name: 'Acme', has_logo: false });
		expect(m.preview).toEqual(typed);
		// A refused draft keeps the last preview.
		previewBranding.mockRejectedValue(new Error('The operator name must be one line of text.'));
		m.draft.operator_name = 'Acme 2';
		await pause();
		expect(m.preview).toEqual(typed);
		stop();
	});

	it('save() sends the trimmed name, then the picked logo, and asks for the stored logo again', async () => {
		saveBranding.mockResolvedValue(server('Acme IT'));
		uploadLogo.mockResolvedValue(server('Acme IT', true));
		const { m, stop } = await setup(server());
		const file = png();
		m.draft.operator_name = ' Acme IT ';
		m.pick(file);
		await settle();
		expect(m.dirty).toBe(true);
		const before = storedLogo.url(true);
		m.submit();
		await settle();
		expect(saveBranding).toHaveBeenCalledWith({ operator_name: 'Acme IT' });
		expect(uploadLogo).toHaveBeenCalledWith(file);
		expect(deleteLogo).not.toHaveBeenCalled();
		expect(m.save.feedback).toEqual({ ok: true, message: 'Saved' });
		expect(m.draft.logo).toEqual({ kind: 'keep' });
		expect(storedLogo.url(true)).not.toBe(before);
		expect(storedLogo.url(false)).toBeNull();
		stop();
	});

	it('"Remove logo" removes a stored logo on save, and only forgets a picked one', async () => {
		saveBranding.mockResolvedValue(server('', true));
		deleteLogo.mockResolvedValue(server());
		const { m, stop } = await setup(server('', true));
		m.removeLogo();
		await settle();
		expect(m.draft.logo).toEqual({ kind: 'remove' });
		m.submit();
		await settle();
		expect(deleteLogo).toHaveBeenCalled();
		expect(uploadLogo).not.toHaveBeenCalled();
		stop();

		const blank = await setup(server());
		blank.m.pick(png());
		blank.m.removeLogo();
		await settle();
		expect(blank.m.draft.logo).toEqual({ kind: 'keep' });
		expect(blank.m.dirty).toBe(false);
		blank.stop();
	});

	it('save() refuses a file of the wrong type before any request', async () => {
		const { m, stop } = await setup(server());
		m.pick(png(100, 'image/gif'));
		await settle();
		expect(m.problem).toBe('The logo must be a PNG, JPEG or WebP image.');
		m.submit();
		await settle();
		expect(saveBranding).not.toHaveBeenCalled();
		expect(uploadLogo).not.toHaveBeenCalled();
		expect(m.save.feedback?.ok).toBe(false);
		stop();
	});
});
