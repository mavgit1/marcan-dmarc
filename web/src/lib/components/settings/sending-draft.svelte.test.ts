// The sending draft behind the Sending section: how the saved group becomes fields and back,
// what counts as a change, why a draft cannot be saved, how the port follows the security, when
// the own login is asked for, and what the save and the test send.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import type { SendingSettingsResponse } from '$lib/types/SendingSettingsResponse';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import {
	effectiveSender,
	problemWith,
	SendingModel,
	type SendingContext,
	type SendingDraft
} from './sending-draft.svelte';

/** Nothing set: off, the mailbox's login. */
const BLANK: SendingDraft = {
	enabled: false,
	host: '',
	port: null,
	security: 'tls',
	same_login_as_mailbox: true,
	username: '',
	sender: '',
	reply_to: '',
	test_recipient: '',
	password: ''
};

vi.mock('$lib/api', () => ({
	getSettings: vi.fn(),
	saveSending: vi.fn(),
	sendTestMail: vi.fn()
}));

const saveSending = vi.mocked(api.saveSending);
const sendTestMail = vi.mocked(api.sendTestMail);

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

const DEFAULTS = { port: 465, starttls_port: 587 };

function server(over: Partial<Omit<SendingSettingsResponse, 'defaults'>> = {}, mailboxUsername = 'dmarc@example.com') {
	return settingsFixture(CONNECTED, {
		mailbox: { username: mailboxUsername },
		sending: {
			effective_sender: mailboxUsername,
			effective_test_recipient: mailboxUsername,
			defaults: DEFAULTS,
			...over
		}
	});
}

/** An enabled group with its own login, as saved. */
const OWN: Partial<SendingSettingsResponse> = {
	enabled: true,
	host: 'smtp.example.net',
	port: 587,
	security: 'starttls',
	same_login_as_mailbox: false,
	username: 'alerts@example.net',
	has_password: true,
	sender: 'alerts@example.net',
	effective_sender: 'alerts@example.net'
};

const draft = (over: Partial<SendingDraft> = {}): SendingDraft => ({
	...BLANK,
	enabled: true,
	host: 'smtp.example.com',
	port: 465,
	...over
});

const CTX: SendingContext = { passwordRequired: false, mailboxUsername: 'dmarc@example.com', mailboxConfigured: true };

async function setup(saved: SettingsResponse | null) {
	let m!: SendingModel;
	const stop = $effect.root(() => {
		m = new SendingModel({ source: { value: () => saved } });
	});
	await settle();
	return { m, stop };
}

beforeEach(() => {
	vi.clearAllMocks();
});

describe('the draft', () => {
	it('is the saved group, with no password and the usual port for an unset one', async () => {
		const blank = await setup(server());
		expect(blank.m.draft).toEqual({ ...BLANK, port: 465 });
		blank.stop();
		const tls = await setup(server({ security: 'starttls' }));
		expect(tls.m.draft.port).toBe(587);
		tls.stop();
		const { m, stop } = await setup(server({ ...OWN, reply_to: 'help@example.net' }));
		expect(m.draft).toEqual({
			enabled: true,
			host: 'smtp.example.net',
			port: 587,
			security: 'starttls',
			same_login_as_mailbox: false,
			username: 'alerts@example.net',
			password: '',
			sender: 'alerts@example.net',
			reply_to: 'help@example.net',
			test_recipient: ''
		});
		stop();
	});

	it('is normalized as typed: trimmed, an empty port as null', async () => {
		const { m, stop } = await setup(server());
		Object.assign(m.draft, { host: ' smtp.example.com ', port: '', sender: ' a@example.com ' });
		expect(m.normalized).toEqual({ ...BLANK, host: 'smtp.example.com', port: null, sender: 'a@example.com' });
		m.draft.port = '587' as unknown as number;
		expect(m.normalized.port).toBe(587);
		stop();
	});

	it('is clean for the saved fields and whitespace; a typed password or a changed field is dirty', async () => {
		const { m, stop } = await setup(server(OWN));
		const change = (over: Partial<SendingDraft>) => {
			m.discard();
			Object.assign(m.draft, over);
			return m.dirty;
		};
		expect(change({ host: ' smtp.example.net ' })).toBe(false);
		expect(change({ password: 'new' })).toBe(true);
		expect(change({ enabled: false })).toBe(true);
		expect(change({ same_login_as_mailbox: true })).toBe(true);
		expect(change({ reply_to: 'help@example.net' })).toBe(true);
		stop();
	});
});

describe('problemWith / effectiveSender', () => {
	it('a complete draft has no problem; with the same login the mailbox address is the sender', () => {
		expect(problemWith(draft(), CTX)).toBeNull();
		expect(effectiveSender(draft(), 'dmarc@example.com')).toBe('dmarc@example.com');
		expect(effectiveSender(draft({ sender: 'a@example.com' }), 'dmarc@example.com')).toBe('a@example.com');
		expect(effectiveSender(draft({ same_login_as_mailbox: false }), 'dmarc@example.com')).toBe('');
	});

	it('names the first missing piece, like the server', () => {
		expect(problemWith(draft({ host: '' }), CTX)).toBe('Enter a server first.');
		expect(problemWith(draft({ port: null }), CTX)).toBe("Enter the SMTP server's port.");
		expect(problemWith(draft(), { ...CTX, mailboxConfigured: false })).toMatch(/^The mailbox has no login yet\./);
		expect(problemWith(draft(), { ...CTX, mailboxUsername: 'dmarc' })).toBe('Enter the sender address.');
		const own = draft({ same_login_as_mailbox: false, sender: 'a@example.com' });
		expect(problemWith(own, CTX)).toBe('Enter the SMTP username.');
		expect(problemWith({ ...own, username: 'u' }, { ...CTX, passwordRequired: true })).toBe('Enter the SMTP password.');
		expect(problemWith({ ...own, username: 'u', password: 'p' }, { ...CTX, passwordRequired: true })).toBeNull();
	});

	it('while off, only the values typed must be valid', () => {
		const off = draft({ enabled: false, host: '', port: null });
		expect(problemWith(off, { ...CTX, mailboxConfigured: false })).toBeNull();
		expect(problemWith({ ...off, port: 70000 }, CTX)).toBe('The SMTP port must be between 1 and 65535.');
		expect(problemWith({ ...off, sender: 'nobody' }, CTX)).toBe(
			'The sender address nobody is not a valid email address.'
		);
		expect(problemWith({ ...off, reply_to: 'help@' }, CTX)).toBe(
			'The Reply-To address help@ is not a valid email address.'
		);
		expect(problemWith({ ...off, test_recipient: 'me' }, CTX)).toBe(
			'The test recipient me is not a valid email address.'
		);
	});
});

describe('SendingModel', () => {
	it('the switch is the draft: switched off, it is an unsaved change saved with the rest, the password kept', async () => {
		saveSending.mockResolvedValue(server({ ...OWN, enabled: false }));
		const { m, stop } = await setup(server(OWN));
		expect(m.feature).toMatchObject({ title: 'Sending', on: true });
		m.feature!.set(false);
		await settle();
		expect(m.feature?.on).toBe(false);
		expect(m.dirty).toBe(true);
		expect(saveSending).not.toHaveBeenCalled();
		m.submit();
		await settle();
		expect(saveSending).toHaveBeenCalledWith(
			expect.objectContaining({ enabled: false, host: 'smtp.example.net', port: 587, username: 'alerts@example.net' })
		);
		expect(saveSending.mock.calls[0][0].password).toBeUndefined();
		expect(m.dirty).toBe(false);
		stop();
	});

	it('switched on without a server, Save waits with the reason', async () => {
		const { m, stop } = await setup(server({ enabled: false, host: '' }));
		expect(m.blocked).toBeNull();
		m.feature!.set(true);
		await settle();
		expect(m.blocked).toBe('Enter a server first.');
		m.draft.host = 'smtp.example.com';
		await settle();
		expect(m.blocked).toBeNull();
		stop();
	});

	it('starts blank, then shows the saved group clean; the own login asks for a password when none is stored', async () => {
		const { m, stop } = await setup(server());
		// The usual port is filled in, and that alone is no unsaved change.
		expect(m.draft).toEqual({ ...BLANK, port: 465 });
		expect(m.dirty).toBe(false);
		expect(m.passwordRequired).toBe(false);
		expect(m.defaultPort).toBe(465);
		// The same login hides the username and password.
		expect(m.ownLogin).toBe(false);

		m.draft.same_login_as_mailbox = false;
		await settle();
		expect(m.ownLogin).toBe(true);
		expect(m.dirty).toBe(true);
		expect(m.passwordRequired).toBe(true);
		stop();
	});

	it('needs the password again once host, port or username change', async () => {
		const { m, stop } = await setup(server(OWN));
		expect(m.passwordRequired).toBe(false);
		m.draft.host = 'SMTP.example.net';
		await settle();
		expect(m.accountChanged).toBe(false);
		m.draft.host = 'smtp.elsewhere.net';
		await settle();
		expect(m.accountChanged).toBe(true);
		expect(m.passwordRequired).toBe(true);
		expect(m.testBlocked).toBe('Enter the SMTP password.');
		stop();
	});

	it('setSecurity() moves an untouched or empty port and keeps a typed one', async () => {
		const { m, stop } = await setup(server(OWN));
		m.setSecurity('tls');
		expect(m.draft).toMatchObject({ security: 'tls', port: 465 });
		m.draft.port = 2525;
		m.setSecurity('starttls');
		expect(m.draft).toMatchObject({ security: 'starttls', port: 2525 });
		m.draft.port = null;
		m.setSecurity('tls');
		expect(m.draft).toMatchObject({ security: 'tls', port: 465 });
		expect(m.defaultPort).toBe(465);
		stop();
	});

	it('the sender domain is the saved effective sender, not what is being typed', async () => {
		const { m, stop } = await setup(server({ sender: 'a@Mail.Example.org', effective_sender: 'a@Mail.Example.org' }));
		expect(m.senderDomain).toBe('mail.example.org');
		m.draft.sender = 'b@other.example';
		await settle();
		expect(m.senderDomain).toBe('mail.example.org');
		stop();

		const none = await setup(server({ effective_sender: '' }, 'dmarc'));
		expect(none.m.senderDomain).toBeNull();
		none.stop();
	});

	it('save() sends the whole group, the password only when typed', async () => {
		const saved = server(OWN);
		saveSending.mockResolvedValue(saved);
		const { m, stop } = await setup(server());
		Object.assign(m.draft, {
			enabled: true,
			host: ' smtp.example.net ',
			port: 587,
			security: 'starttls',
			same_login_as_mailbox: false,
			username: 'alerts@example.net',
			password: ' secret ',
			sender: 'alerts@example.net'
		});
		await settle();
		m.submit();
		await settle();
		expect(saveSending).toHaveBeenCalledWith({
			enabled: true,
			host: 'smtp.example.net',
			port: 587,
			security: 'starttls',
			same_login_as_mailbox: false,
			username: 'alerts@example.net',
			password: ' secret ',
			sender: 'alerts@example.net',
			reply_to: '',
			test_recipient: ''
		});
		expect(m.save.feedback).toEqual({ ok: true, message: 'Saved' });
		expect(m.dirty).toBe(false);
		stop();
	});

	it("save() sends what is typed; the server's refusal is the feedback", async () => {
		const { m, stop } = await setup(server());
		m.draft.sender = 'nobody';
		await settle();
		const refusal = 'The sender address nobody is not a valid email address.';
		saveSending.mockRejectedValueOnce(new Error(refusal));
		m.submit();
		await settle();
		expect(saveSending).toHaveBeenCalledWith(expect.objectContaining({ sender: 'nobody' }));
		expect(m.save.feedback).toEqual({ ok: false, message: refusal });
		stop();
	});

	it('the test sends the values on screen before they are saved, to the test recipient on screen or the sender', async () => {
		const blank = await setup(server({ enabled: false, host: '' }));
		expect(blank.m.testBlocked).toBe('Turn on sending to send a test.');
		blank.m.feature!.set(true);
		await settle();
		expect(blank.m.testBlocked).toBe('Enter a server first.');
		blank.stop();

		const { m, stop } = await setup(server({ ...OWN, enabled: false, has_password: true }));
		expect(m.testBlocked).toBe('Turn on sending to send a test.');
		m.feature!.set(true);
		await settle();
		expect(m.testBlocked).toBeNull();
		expect(m.testRecipient).toBe('alerts@example.net');
		m.draft.sender = 'other@example.net';
		await settle();
		expect(m.testBlocked).toBeNull();
		expect(m.testRecipient).toBe('other@example.net');
		m.draft.host = 'smtp.elsewhere.net';
		await settle();
		expect(m.testBlocked).toBe('Enter the SMTP password.');
		m.draft.password = 'typed';
		m.draft.test_recipient = 'nobody';
		await settle();
		expect(m.testBlocked).toBe('The test recipient nobody is not a valid email address.');

		sendTestMail.mockResolvedValueOnce({ ok: true, message: 'Test message sent to owner@example.org.' });
		m.draft.test_recipient = ' owner@example.org ';
		await settle();
		expect(m.testBlocked).toBeNull();
		expect(m.testRecipient).toBe('owner@example.org');
		await m.sendTest();
		expect(sendTestMail).toHaveBeenLastCalledWith({
			sending: expect.objectContaining({
				host: 'smtp.elsewhere.net',
				password: 'typed',
				sender: 'other@example.net',
				test_recipient: 'owner@example.org'
			})
		});
		expect(m.test.feedback).toEqual({ ok: true, message: 'Test message sent to owner@example.org.' });

		sendTestMail.mockRejectedValueOnce(new Error('boom'));
		m.discard();
		await settle();
		await m.sendTest();
		expect(sendTestMail).toHaveBeenLastCalledWith({
			sending: expect.objectContaining({ host: 'smtp.example.net', password: undefined, test_recipient: '' })
		});
		expect(m.test.feedback?.ok).toBe(false);
		stop();
	});
});
