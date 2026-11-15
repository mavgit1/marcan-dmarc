// The account model behind the account form: what a save does (a changed login tested first,
// every step passed and the folder there, then the whole account), how a new account reports its
// id, how ports follow the security, which test answer counts, which reasons the Test row says
// beside the field notes, and the refusals it holds client-side in the server's words.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { accountFixture, CONNECTED, settingsFixture } from '$lib/test/settings-fixture';
import type { AccountTestResponse } from '$lib/types/AccountTestResponse';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import { ACCOUNT_TEXT, AccountModel, fromAccount, MS_TEXT } from './account-draft.svelte';

vi.mock('$lib/api', () => ({
	createAccount: vi.fn(),
	updateAccount: vi.fn(),
	testAccount: vi.fn(),
	generateCertificate: vi.fn(),
	getSettings: vi.fn(),
	getMeta: vi.fn(() => Promise.resolve(null)),
	getSystemHealth: vi.fn(() => Promise.resolve(null)),
	syncAll: vi.fn()
}));

const createAccount = vi.mocked(api.createAccount);
const updateAccount = vi.mocked(api.updateAccount);
const testAccount = vi.mocked(api.testAccount);

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

/** Every step passed, the folder there. */
function passed(over: Partial<AccountTestResponse> = {}): AccountTestResponse {
	return {
		token: null,
		mailbox: {
			ok: true,
			message: 'Logged in as dmarc@example.com.',
			detail: null,
			docs: null,
			folders: [
				{ name: 'INBOX', delimiter: '/' },
				{ name: 'DMARC', delimiter: '/' }
			],
			selected: { name: 'INBOX', exists: true, messages: 3 }
		},
		smtp: { ok: true, message: 'Logged in as dmarc@example.com.', detail: null, docs: null },
		...over
	};
}

const step = (ok: boolean, message: string) => ({ ok, message, detail: null, docs: null });

async function setup(saved: SettingsResponse, id: number | null, onCreated?: (id: number) => void) {
	let m!: AccountModel;
	const stop = $effect.root(() => {
		m = new AccountModel({ source: { value: () => saved }, id, onCreated });
	});
	await settle();
	return { m, stop };
}

beforeEach(() => {
	vi.clearAllMocks();
});

describe('AccountModel.failure', () => {
	const d = fromAccount(accountFixture(), settingsFixture(CONNECTED));

	it('names the first failed step, then a missing folder of a receiving account', () => {
		expect(AccountModel.failure(passed(), d)).toBeNull();
		expect(AccountModel.failure(passed({ token: step(false, 'No token.') }), d)).toBe('No token.');
		expect(
			AccountModel.failure(
				passed({
					mailbox: { ...passed().mailbox!, ok: false, message: 'Login rejected.' },
					smtp: step(false, 'No SMTP.')
				}),
				d
			)
		).toBe('Login rejected.');
		expect(AccountModel.failure(passed({ smtp: step(false, 'No SMTP.') }), d)).toBe('No SMTP.');
		const gone = passed({
			mailbox: { ...passed().mailbox!, selected: { name: 'Gone', exists: false, messages: null } }
		});
		expect(AccountModel.failure(gone, d)).toBe('Folder "Gone" was not found on the server.');
		expect(AccountModel.failure(gone, { ...d, receive: false })).toBeNull();
	});
});

/** A Microsoft 365 account whose certificate expired on 1 January 2020. */
const EXPIRED = accountFixture({
	provider: 'microsoft365',
	login: {
		kind: 'microsoft',
		tenant_id: '11111111-2222-3333-4444-555555555555',
		client_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
		certificate: { thumbprint: 'AB12', expires_at: Date.parse('2020-01-01T23:59:59Z') / 1000 }
	},
	imap: null,
	smtp: null
});

describe('AccountModel', () => {
	it('renames an account whose certificate expired, without a test', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED, { accounts: [EXPIRED] }), 1);
		m.draft.name = 'Renamed';
		await settle();
		expect(m.microsoftProblem).toBeNull();
		expect(m.invalid).toBeNull();
		updateAccount.mockResolvedValue(settingsFixture({ accounts: [{ ...EXPIRED, name: 'Renamed' }] }));
		expect(await m.persist()).toEqual({ ok: true, message: 'Saved' });
		expect(testAccount).not.toHaveBeenCalled();
		expect(updateAccount).toHaveBeenCalledWith(1, expect.objectContaining({ imap: null, smtp: null }));
		stop();
	});

	it('saves a new Microsoft account with a generated certificate and nothing else, untested', async () => {
		const created: number[] = [];
		const { m, stop } = await setup(settingsFixture(), null, (id) => created.push(id));
		m.setProvider('microsoft365');
		m.setAddress('dmarc@example.com');
		Object.assign(m.draft, {
			tenantId: '11111111-2222-3333-4444-555555555555',
			clientId: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'
		});
		await settle();
		expect(m.invalid).toEqual({ field: 'certificate', message: MS_TEXT.certificate });
		const certificate = { draft_id: 'd1', thumbprint: 'AB12', expires_at: 1_900_000_000, subject: 'dmarc@example.com' };
		vi.mocked(api.generateCertificate).mockResolvedValue(certificate);
		await m.generateCertificate();
		expect(api.generateCertificate).toHaveBeenCalledWith({ subject: 'dmarc@example.com' });
		await settle();
		expect(m.invalid).toBeNull();
		const stored = accountFixture({ ...EXPIRED, id: 4, name: 'dmarc@example.com' });
		createAccount.mockResolvedValue(settingsFixture({ accounts: [stored] }));
		expect(await m.persist()).toEqual({
			ok: true,
			message: 'Saved: upload the certificate to the app registration, then test'
		});
		expect(testAccount).not.toHaveBeenCalled();
		expect(createAccount).toHaveBeenCalledWith({
			name: '',
			provider: 'microsoft365',
			address: 'dmarc@example.com',
			login: {
				kind: 'microsoft',
				tenant_id: '11111111-2222-3333-4444-555555555555',
				client_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
				draft_id: 'd1'
			},
			imap: null,
			smtp: null,
			receiving: { enabled: true, folder: 'INBOX', forwarded_from: [] }
		});
		// It stays on the saved account, so the .cer can be downloaded.
		expect(created).toEqual([4]);
		expect(m.account?.id).toBe(4);
		stop();
	});

	it('saves a change that is no login without a test', async () => {
		const saved = settingsFixture(CONNECTED);
		const { m, stop } = await setup(saved, 1);
		m.draft.name = 'DMARC reports';
		updateAccount.mockResolvedValue(settingsFixture({ accounts: [accountFixture({ name: 'DMARC reports' })] }));
		const feedback = await m.persist();
		expect(feedback).toEqual({ ok: true, message: 'Saved' });
		expect(testAccount).not.toHaveBeenCalled();
		expect(updateAccount).toHaveBeenCalledWith(1, expect.objectContaining({ name: 'DMARC reports' }));
		expect(m.account?.name).toBe('DMARC reports');
		stop();
	});

	it('saves a changed login only after a test of that login passed', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		m.draft.imapHost = 'imap.example.net';
		m.draft.password = 'secret';
		testAccount.mockResolvedValue(passed({ mailbox: { ...passed().mailbox!, ok: false, message: 'Login rejected.' } }));
		expect(await m.persist()).toEqual({ ok: false, message: 'Login rejected.' });
		expect(updateAccount).not.toHaveBeenCalled();
		expect(testAccount).toHaveBeenCalledWith(
			expect.objectContaining({
				id: 1,
				account: expect.objectContaining({ imap: { host: 'imap.example.net', port: 993, security: 'tls' } }),
				folder: 'INBOX',
				smtp: true
			})
		);

		testAccount.mockResolvedValue(passed());
		updateAccount.mockResolvedValue(settingsFixture(CONNECTED));
		expect(await m.persist()).toEqual({ ok: true, message: 'Tested and saved' });
		expect(updateAccount).toHaveBeenCalledWith(
			1,
			expect.objectContaining({ login: { kind: 'password', username: 'dmarc@example.com', password: 'secret' } })
		);
		stop();
	});

	it('tests a Microsoft login without an SMTP line, and saves it without any server', async () => {
		const microsoft = accountFixture({
			...EXPIRED,
			login: {
				kind: 'microsoft',
				tenant_id: '11111111-2222-3333-4444-555555555555',
				client_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee',
				certificate: { thumbprint: 'AB12', expires_at: null }
			}
		});
		const { m, stop } = await setup(settingsFixture(CONNECTED, { accounts: [microsoft] }), 1);
		// It reads and sends over Graph: no server is needed, nor asked for.
		m.draft.receive = false;
		await settle();
		expect(m.invalid).toBeNull();
		expect(m.testProblem).toBeNull();
		expect(m.loginChanged).toBe(true);
		const tokenOnly: AccountTestResponse = {
			token: step(true, 'Microsoft issued a token for the app registration.'),
			mailbox: null,
			smtp: null
		};
		testAccount.mockResolvedValue(tokenOnly);
		updateAccount.mockResolvedValue(settingsFixture(CONNECTED, { accounts: [microsoft] }));
		expect(await m.persist()).toEqual({ ok: true, message: 'Tested and saved' });
		expect(testAccount).toHaveBeenCalledWith(expect.objectContaining({ smtp: false }));
		expect(updateAccount).toHaveBeenCalled();

		// Receiving, still no SMTP line, and no servers sent.
		m.draft.receive = true;
		await m.testLogin();
		expect(testAccount).toHaveBeenLastCalledWith(
			expect.objectContaining({ smtp: false, account: expect.objectContaining({ imap: null, smtp: null }) })
		);
		stop();
	});

	it('switches receiving off without a test, and on only after one passed', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		m.draft.receive = false;
		const off = accountFixture({ receiving: { enabled: false, folder: 'INBOX', forwarded_from: [] } });
		updateAccount.mockResolvedValue(settingsFixture({ accounts: [off] }));
		expect(m.loginChanged).toBe(false);
		expect(await m.persist()).toEqual({ ok: true, message: 'Saved' });
		expect(testAccount).not.toHaveBeenCalled();
		expect(updateAccount).toHaveBeenCalledWith(
			1,
			expect.objectContaining({ receiving: expect.objectContaining({ enabled: false }) })
		);
		stop();

		const stopped = await setup(settingsFixture({ accounts: [off] }), 1);
		stopped.m.draft.receive = true;
		expect(stopped.m.loginChanged).toBe(true);
		testAccount.mockResolvedValue(passed({ mailbox: { ...passed().mailbox!, ok: false, message: 'Login rejected.' } }));
		updateAccount.mockClear();
		expect(await stopped.m.persist()).toEqual({ ok: false, message: 'Login rejected.' });
		expect(updateAccount).not.toHaveBeenCalled();
		stopped.stop();
	});

	it('refuses a folder the server does not have', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		m.draft.folder = 'Nope';
		testAccount.mockResolvedValue(
			passed({ mailbox: { ...passed().mailbox!, selected: { name: 'Nope', exists: false, messages: null } } })
		);
		expect(await m.persist()).toEqual({ ok: false, message: 'Folder "Nope" was not found on the server.' });
		expect(testAccount).toHaveBeenCalledWith(expect.objectContaining({ folder: 'Nope' }));
		expect(updateAccount).not.toHaveBeenCalled();
		stop();
	});

	it('a new account reports its id once stored', async () => {
		const created: number[] = [];
		const { m, stop } = await setup(settingsFixture(), null, (id) => created.push(id));
		m.setProvider('generic');
		Object.assign(m.draft, { imapHost: 'imap.example.com', smtpHost: '', password: 'secret' });
		m.setAddress('dmarc@example.com');
		testAccount.mockResolvedValue(passed({ smtp: null }));
		createAccount.mockResolvedValue(settingsFixture({ accounts: [accountFixture({ id: 7 })] }));
		expect(await m.persist()).toEqual({ ok: true, message: 'Tested and saved' });
		expect(createAccount).toHaveBeenCalledWith(
			expect.objectContaining({
				address: 'dmarc@example.com',
				login: expect.objectContaining({ username: 'dmarc@example.com' })
			})
		);
		expect(created).toEqual([7]);
		expect(m.id).toBe(7);
		expect(m.account?.id).toBe(7);
		stop();
	});

	it('moves an untouched port with the security and keeps a typed one', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		m.setImapSecurity('starttls');
		expect(m.draft).toMatchObject({ imapSecurity: 'starttls', imapPort: 143 });
		m.setImapSecurity('tls');
		expect(m.draft.imapPort).toBe(993);
		m.draft.imapPort = 1993;
		m.setImapSecurity('starttls');
		expect(m.draft.imapPort).toBe(1993);

		m.setSmtpSecurity('starttls');
		expect(m.draft).toMatchObject({ smtpSecurity: 'starttls', smtpPort: 587 });
		m.draft.smtpPort = 2525;
		m.setSmtpSecurity('tls');
		expect(m.draft).toMatchObject({ smtpSecurity: 'tls', smtpPort: 2525 });
		stop();
	});

	it('takes the answer of the latest test; an earlier one that ends later is dropped', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		let first!: (v: AccountTestResponse) => void;
		testAccount.mockReturnValueOnce(new Promise((res) => (first = res)));
		testAccount.mockResolvedValueOnce(passed({ smtp: step(true, 'second') }));
		const p1 = m.testLogin('INBOX');
		const p2 = m.testLogin('DMARC');
		expect((await p2)?.smtp?.message).toBe('second');
		first(passed({ smtp: step(true, 'first') }));
		expect(await p1).toBeNull();
		expect(m.result?.smtp?.message).toBe('second');
		expect(m.testing).toBe(false);
		expect(m.tested).toBe(true);

		// A failed request is a failed mailbox step.
		testAccount.mockRejectedValueOnce(new Error('down'));
		const res = await m.testLogin();
		expect(res?.mailbox).toEqual({
			ok: false,
			message: 'down',
			detail: null,
			docs: null,
			folders: [],
			selected: null
		});
		stop();
	});

	it('says why the test waits under Test only when no field note says it', async () => {
		const { m, stop } = await setup(settingsFixture(CONNECTED), 1);
		expect(m.testNote).toBeNull();
		// Another username: the password must be typed again, said at the password field.
		m.draft.username = 'other@example.com';
		await settle();
		expect(m.testProblem).toBe(ACCOUNT_TEXT.passwordAgain);
		expect(m.fieldProblem('password')).toBe(ACCOUNT_TEXT.passwordAgain);
		expect(m.testNote).toBeNull();
		// No server at all: no field says it, the Test row does.
		Object.assign(m.draft, { username: 'dmarc@example.com', imapHost: '', smtpHost: '', receive: false });
		await settle();
		expect(m.testNote).toBe(ACCOUNT_TEXT.serverToTest);
		stop();

		// A Microsoft form nothing was typed into yet: no field note shows, so the Test row says it.
		const fresh = await setup(settingsFixture(), null);
		fresh.m.setProvider('microsoft365');
		await settle();
		expect(fresh.m.draft.loginKind).toBe('microsoft');
		expect(fresh.m.edited).toBe(false);
		expect(fresh.m.testNote).toBe(fresh.m.testProblem);
		expect(fresh.m.testNote).not.toBeNull();
		fresh.stop();
	});

	it("holds a forwarding address that is the own one or another account's, and an address another account uses", async () => {
		const other = accountFixture({
			id: 2,
			name: 'Second',
			address: 'second@example.com',
			receiving: { enabled: true, folder: 'INBOX', forwarded_from: ['dmarc@client.example'] }
		});
		const saved = settingsFixture({ accounts: [accountFixture(), other] });
		const { m, stop } = await setup(saved, 1);
		m.draft.forwardedFrom = ['  ', 'DMARC@example.com'];
		await settle();
		expect(m.invalid).toEqual({
			field: 'forwardedFrom:dmarc@example.com',
			message: "The forwarding address DMARC@example.com is the account's own address."
		});
		m.draft.forwardedFrom = ['Dmarc@Client.example'];
		await settle();
		expect(m.invalid).toEqual({
			field: 'forwardedFrom:dmarc@client.example',
			message: 'Another account already uses Dmarc@Client.example.'
		});
		m.draft.forwardedFrom = ['client.example'];
		await settle();
		expect(m.invalid?.message).toBe('The forwarding address client.example is not a valid email address.');
		m.draft.forwardedFrom = [];
		// An address another account forwards is taken too.
		m.setAddress('dmarc@client.example');
		await settle();
		expect(m.invalid).toEqual({ field: 'address', message: 'Another account already uses dmarc@client.example.' });
		m.setAddress('Second@Example.com');
		await settle();
		expect(m.invalid).toEqual({ field: 'address', message: 'Another account already uses Second@Example.com.' });
		// Its own stored address is no clash.
		m.setAddress('dmarc@example.com');
		await settle();
		expect(m.invalid).toBeNull();
		stop();
	});
});
