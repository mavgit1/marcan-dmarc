// The alerts draft behind the Alerts section: how the saved group becomes fields and back, what
// counts as a change, why a draft cannot be saved, and what the save and the test alert send.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SENDING_ON, settingsFixture } from '$lib/test/settings-fixture';
import type { AlertSettings } from '$lib/types/AlertSettings';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import { AlertsModel, recipientList, withMailed } from './alerts-draft.svelte';

vi.mock('$lib/api', () => ({
	getSettings: vi.fn(),
	saveAlerts: vi.fn(),
	sendTestAlert: vi.fn()
}));

const saveAlerts = vi.mocked(api.saveAlerts);
const sendTestAlert = vi.mocked(api.sendTestAlert);

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

function server(alerts: Partial<AlertSettings> = {}, sendingOn = true): SettingsResponse {
	return settingsFixture(SENDING_ON, { sending: { enabled: sendingOn }, alerts });
}

const ON: Partial<AlertSettings> = {
	enabled: true,
	recipients: ['ops@example.com', 'noc@example.com'],
	digest: 'weekly',
	digest_hour: 18,
	digest_weekday: 'friday',
	base_url: 'https://marcan.example.com'
};

async function setup(saved: SettingsResponse | null) {
	let m!: AlertsModel;
	const stop = $effect.root(() => {
		m = new AlertsModel({ source: { value: () => saved } });
	});
	await settle();
	return { m, stop };
}

beforeEach(() => {
	vi.clearAllMocks();
});

describe('the draft', () => {
	it('is the saved group, the recipients one per line', async () => {
		const { m, stop } = await setup(server(ON));
		expect(m.draft).toEqual({
			enabled: true,
			recipients: 'ops@example.com\nnoc@example.com',
			digest: 'weekly',
			digest_hour: 18,
			digest_weekday: 'friday',
			base_url: 'https://marcan.example.com',
			muted_kinds: []
		});
		stop();
	});

	it('reads recipients typed any way, without repeats', async () => {
		expect(recipientList(' ops@example.com, noc@example.com\n\nOPS@example.com;x@example.org ')).toEqual([
			'ops@example.com',
			'noc@example.com',
			'x@example.org'
		]);
		const { m, stop } = await setup(server());
		m.draft.base_url = ' https://marcan.example.com/ ';
		expect(m.normalized.base_url).toBe('https://marcan.example.com');
		stop();
	});

	it('is clean for the saved values written differently; a changed field is dirty', async () => {
		const { m, stop } = await setup(server(ON));
		const change = (over: Partial<typeof m.draft>) => {
			m.discard();
			Object.assign(m.draft, over);
			return m.dirty;
		};
		expect(change({ recipients: 'ops@example.com, noc@example.com' })).toBe(false);
		expect(change({ base_url: 'https://marcan.example.com/' })).toBe(false);
		expect(change({ digest_hour: 6 })).toBe(true);
		expect(change({ recipients: 'ops@example.com' })).toBe(true);
		stop();
	});
});

describe('withMailed', () => {
	it('mutes and unmutes, sorted and once each', () => {
		expect(withMailed([], ['b', 'a'], false)).toEqual(['a', 'b']);
		expect(withMailed(['a', 'b'], ['a', 'c'], true)).toEqual(['b']);
		expect(withMailed(['a'], ['a'], false)).toEqual(['a']);
	});

	it('a muted list in another order is no change', async () => {
		const { m, stop } = await setup(server({ muted_kinds: ['b', 'a'] }));
		expect(m.draft.muted_kinds).toEqual(['a', 'b']);
		m.draft.muted_kinds = ['b', 'a'];
		expect(m.dirty).toBe(false);
		m.draft.muted_kinds = ['a'];
		expect(m.dirty).toBe(true);
		stop();
	});
});

describe('AlertsModel', () => {
	it('the switch is the draft: flipped, it is an unsaved change saved with the rest, the values kept', async () => {
		saveAlerts.mockResolvedValue(server({ ...ON, enabled: false }));
		const { m, stop } = await setup(server(ON));
		expect(m.feature).toMatchObject({ title: 'Send alerts', on: true });
		m.feature!.set(false);
		await settle();
		expect(m.feature?.on).toBe(false);
		expect(m.draft.enabled).toBe(false);
		expect(m.dirty).toBe(true);
		expect(m.blocked).toBeNull();
		expect(saveAlerts).not.toHaveBeenCalled();
		m.submit();
		await settle();
		expect(saveAlerts).toHaveBeenCalledWith(
			expect.objectContaining({ enabled: false, digest: 'weekly', recipients: ['ops@example.com', 'noc@example.com'] })
		);
		stop();
	});

	it('switched on without a recipient, Save waits with the reason', async () => {
		const { m, stop } = await setup(server({ enabled: false, recipients: [] }));
		expect(m.blocked).toBeNull();
		m.feature!.set(true);
		await settle();
		expect(m.blocked).toBe('Add at least one recipient first.');
		m.draft.recipients = 'ops@example.com';
		await settle();
		expect(m.blocked).toBeNull();
		stop();
	});

	it('shows the saved group clean, and names a typed recipient that is no address', async () => {
		const { m, stop } = await setup(server(ON));
		expect(m.draft.recipients).toBe('ops@example.com\nnoc@example.com');
		expect(m.dirty).toBe(false);
		m.draft.recipients = 'ops@example.com\nbad';
		await settle();
		expect(m.badRecipient).toBe('bad');
		expect(m.dirty).toBe(true);
		stop();
	});

	it('save() sends the whole group with the recipients as a list', async () => {
		saveAlerts.mockResolvedValue(server(ON));
		const { m, stop } = await setup(server());
		Object.assign(m.draft, {
			enabled: true,
			recipients: 'ops@example.com, noc@example.com',
			digest: 'weekly',
			digest_hour: 18,
			digest_weekday: 'friday',
			base_url: 'https://marcan.example.com/'
		});
		await settle();
		m.submit();
		await settle();
		expect(saveAlerts).toHaveBeenCalledWith({
			enabled: true,
			recipients: ['ops@example.com', 'noc@example.com'],
			digest: 'weekly',
			digest_hour: 18,
			digest_weekday: 'friday',
			base_url: 'https://marcan.example.com',
			muted_kinds: []
		});
		expect(m.save.feedback).toEqual({ ok: true, message: 'Saved' });
		stop();
	});

	it('"Alert kinds": an entry or a whole group on and off, saved as the muted ids', async () => {
		saveAlerts.mockResolvedValue(server(ON));
		const { m, stop } = await setup(server({ ...ON, muted_kinds: ['policy_changed'] }));
		expect(m.isMailed('policy_changed')).toBe(false);
		expect(m.isMailed('health_changed')).toBe(true);
		m.setMailed(['dns_record_changed.spf', 'dns_record_changed.dkim'], false);
		m.setMailed(['policy_changed'], true);
		await settle();
		expect(m.draft.muted_kinds).toEqual(['dns_record_changed.dkim', 'dns_record_changed.spf']);
		expect(m.dirty).toBe(true);
		m.submit();
		await settle();
		expect(saveAlerts).toHaveBeenCalledWith(
			expect.objectContaining({ muted_kinds: ['dns_record_changed.dkim', 'dns_record_changed.spf'] })
		);
		stop();
	});

	it("save() sends what is typed; the server's refusal is the feedback", async () => {
		const { m, stop } = await setup(server(ON));
		m.draft.base_url = 'marcan.example.com';
		await settle();
		const refusal =
			'The base URL marcan.example.com is not a web address: write it as it opens Marcan, such as https://marcan.example.com.';
		saveAlerts.mockRejectedValueOnce(new Error(refusal));
		m.submit();
		await settle();
		expect(saveAlerts).toHaveBeenCalledWith(expect.objectContaining({ base_url: 'marcan.example.com' }));
		expect(m.save.feedback).toEqual({ ok: false, message: refusal });
		expect(m.dirty).toBe(true);
		stop();
	});

	it('the test goes to the test recipient saved under Sending, and needs the switch and sending on', async () => {
		const off = await setup(server(ON, false));
		expect(off.m.testBlocked).toBe('Turn on sending to send a test.');
		off.stop();

		const nowhere = server(ON);
		nowhere.sending.effective_test_recipient = '';
		const none = await setup(nowhere);
		expect(none.m.testBlocked).toBe('Set a test recipient under Sending first.');
		none.stop();

		// Nobody typed as a recipient is no reason to wait: the test does not go to them.
		const nobody = await setup(server());
		expect(nobody.m.testBlocked).toBe('Turn on alert emails to send a test.');
		nobody.m.draft.enabled = true;
		await settle();
		expect(nobody.m.testBlocked).toBeNull();
		nobody.stop();

		const { m, stop } = await setup(server(ON));
		expect(m.testRecipient).toBe('dmarc@example.com');
		// The recipients and the base URL are the server's to check; the test does not wait for them.
		m.draft.recipients = 'team@example.com\nnobody';
		await settle();
		expect(m.testBlocked).toBeNull();
		expect(m.badRecipient).toBe('nobody');

		sendTestAlert.mockResolvedValueOnce({ ok: true, message: 'Test alert sent to dmarc@example.com.' });
		await m.sendTest();
		expect(sendTestAlert).toHaveBeenLastCalledWith({
			alerts: expect.objectContaining({ base_url: 'https://marcan.example.com' })
		});
		expect(m.test.feedback).toEqual({ ok: true, message: 'Test alert sent to dmarc@example.com.' });
		sendTestAlert.mockRejectedValueOnce(new Error('boom'));
		await m.sendTest();
		expect(m.test.feedback?.ok).toBe(false);
		stop();
	});
});
