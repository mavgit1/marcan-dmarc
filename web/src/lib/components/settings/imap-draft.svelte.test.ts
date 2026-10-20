// The mailbox draft behind the IMAP forms: how a saved mailbox becomes fields and back, what
// counts as a change, why a draft cannot be saved, and what each save sends.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import type { ConnectImapResponse } from '$lib/types/ConnectImapResponse';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import {
	BLANK,
	connectionKey,
	effectiveAddress,
	fromServer,
	ImapModel,
	normalize,
	problemWith,
	type ImapDraft
} from './imap-draft.svelte';

vi.mock('$lib/api', () => ({
	connectImap: vi.fn(),
	getSettings: vi.fn(),
	saveMailbox: vi.fn(),
	saveReporting: vi.fn(),
	saveMonitoring: vi.fn(),
	getMeta: vi.fn(() => Promise.resolve(null)),
	getSystemHealth: vi.fn(() => Promise.resolve(null)),
	syncImap: vi.fn()
}));

const connectImap = vi.mocked(api.connectImap);
const saveMailbox = vi.mocked(api.saveMailbox);
const saveReporting = vi.mocked(api.saveReporting);
const saveMonitoring = vi.mocked(api.saveMonitoring);

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

/** The fields of the three groups the draft covers, flat, for overriding one in a test. */
type Saved = SettingsResponse['mailbox'] & SettingsResponse['reporting'] & SettingsResponse['monitoring'];

function server(over: Partial<Omit<Saved, 'defaults'>> = {}): SettingsResponse {
	const f = {
		host: 'imap.example.com',
		port: 993,
		security: 'tls' as const,
		username: 'dmarc@example.com',
		folder: 'INBOX',
		poll_interval_secs: 600,
		has_password: true,
		configured: true,
		report_address: 'dmarc@example.com',
		report_org_domain: 'example.com',
		reporting_mode: 'direct' as const,
		reporting_mode_saved: true,
		report_freshness_warning_hours: 48,
		domain_reports_stopped_days: 7,
		...over
	};
	return settingsFixture({
		mailbox: {
			host: f.host,
			port: f.port,
			security: f.security,
			username: f.username,
			folder: f.folder,
			poll_interval_secs: f.poll_interval_secs,
			has_password: f.has_password,
			configured: f.configured,
			defaults: { port: 993, starttls_port: 143, folder: 'INBOX', poll_interval_secs: 600 }
		},
		reporting: {
			report_address: f.report_address,
			report_org_domain: f.report_org_domain,
			reporting_mode: f.reporting_mode,
			reporting_mode_saved: f.reporting_mode_saved
		},
		monitoring: {
			report_freshness_warning_hours: f.report_freshness_warning_hours,
			domain_reports_stopped_days: f.domain_reports_stopped_days
		},
		sending: { effective_sender: f.username, effective_test_recipient: f.username }
	});
}

function probe(over: Partial<ConnectImapResponse> = {}): ConnectImapResponse {
	return {
		ok: true,
		message: 'Connected',
		detail: null,
		folders: [{ name: 'INBOX', delimiter: '/' }],
		selected: { name: 'INBOX', exists: true, messages: 3 },
		...over
	};
}

async function setup(saved: SettingsResponse | null, save?: ImapModel['saveSettings']) {
	let m!: ImapModel;
	const stop = $effect.root(() => {
		m = new ImapModel({
			source: { value: () => saved },
			save: (d, s) => (save ?? ((d, s) => m.saveSettings(d, s)))(d, s)
		});
	});
	await settle();
	return { m, stop };
}

beforeEach(() => {
	vi.clearAllMocks();
});

describe('fromServer / normalize', () => {
	it('turns the saved mailbox into fields: no password, minutes, the login address as ""', () => {
		expect(fromServer(server())).toEqual({
			host: 'imap.example.com',
			port: 993,
			security: 'tls',
			username: 'dmarc@example.com',
			password: '',
			reportAddress: '',
			folder: 'INBOX',
			mode: 'direct',
			pollMinutes: 10,
			warnHours: 48,
			stoppedDays: 7
		});
		expect(fromServer(server({ report_address: 'Reports@Example.com', poll_interval_secs: 20 })).reportAddress).toBe(
			'Reports@Example.com'
		);
		expect(fromServer(server({ poll_interval_secs: 20 })).pollMinutes).toBe(1);
	});

	it('normalizes what was typed: trimmed, numbers, the login address as ""', () => {
		const d: ImapDraft = {
			...BLANK,
			host: ' imap.example.com ',
			port: '993' as unknown as number,
			username: ' DMARC@example.com ',
			reportAddress: ' dmarc@EXAMPLE.com ',
			pollMinutes: '5' as unknown as number,
			warnHours: '24' as unknown as number
		};
		expect(normalize(d)).toMatchObject({
			host: 'imap.example.com',
			port: 993,
			username: 'DMARC@example.com',
			reportAddress: '',
			pollMinutes: 5,
			warnHours: 24
		});
		expect(normalize({ ...d, reportAddress: 'other@example.com' }).reportAddress).toBe('other@example.com');
	});
});

describe('security and port', () => {
	const defaults = server().mailbox.defaults;

	it('fills an unset port with the usual one for the security', () => {
		const unset = (security: 'tls' | 'starttls') => server({ port: 0, security });
		expect(fromServer(unset('tls')).port).toBe(defaults.port);
		expect(fromServer(unset('starttls')).port).toBe(defaults.starttls_port);
	});
});

describe('dirty', () => {
	/** Whether the saved mailbox with `over` typed is an unsaved change. */
	async function dirtyWith(over: Partial<ImapDraft>): Promise<boolean> {
		const { m, stop } = await setup(server());
		Object.assign(m.draft, over);
		const dirty = m.dirty;
		stop();
		return dirty;
	}

	it('is clean for the saved fields, whitespace and the login typed as report address', async () => {
		expect(await dirtyWith({})).toBe(false);
		expect(await dirtyWith({ host: ' imap.example.com ' })).toBe(false);
		expect(await dirtyWith({ reportAddress: 'dmarc@example.com' })).toBe(false);
	});

	it('a typed password or any other changed field is dirty', async () => {
		expect(await dirtyWith({ password: 'x' })).toBe(true);
		expect(await dirtyWith({ folder: 'Reports' })).toBe(true);
		expect(await dirtyWith({ mode: 'forwarded' })).toBe(true);
		expect(await dirtyWith({ security: 'starttls' })).toBe(true);
		expect(await dirtyWith({ warnHours: 24 })).toBe(true);
	});
});

describe('problemWith / effectiveAddress', () => {
	const ok = fromServer(server());

	it('a complete draft has no problem; the login is the report address', () => {
		expect(problemWith(ok, false)).toBeNull();
		expect(effectiveAddress(ok)).toBe('dmarc@example.com');
		expect(effectiveAddress({ ...ok, reportAddress: 'r@example.net' })).toBe('r@example.net');
		expect(effectiveAddress({ ...ok, username: 'dmarc' })).toBe('');
	});

	it('names the first missing field, in form order', () => {
		expect(problemWith({ ...ok, host: '' }, false)).toBe('Enter the server host.');
		expect(problemWith({ ...ok, port: 0 }, false)).toBe('Enter the server port.');
		expect(problemWith({ ...ok, username: '' }, false)).toBe('Enter the username.');
		expect(problemWith(ok, true)).toBe('Enter the password.');
		expect(problemWith({ ...ok, password: ' ' }, true)).toBe('Enter the password.');
		expect(problemWith({ ...ok, reportAddress: 'nope' }, false)).toBe(
			'The report address is not a valid email address.'
		);
		expect(problemWith({ ...ok, username: 'dmarc' }, false)).toBe(
			'The login is not an email address: enter the report address.'
		);
		expect(problemWith({ ...ok, username: 'dmarc', reportAddress: 'r@example.com' }, false)).toBeNull();
		expect(problemWith({ ...ok, pollMinutes: 0 }, false)).toBe('Check the mailbox at least every minute.');
	});
});

describe('ImapModel', () => {
	it('starts blank, then shows the saved mailbox clean; an edit is dirty, a password changes the mailbox', async () => {
		const { m, stop } = await setup(server());
		expect(m.saved?.mailbox.host).toBe('imap.example.com');
		expect(m.dirty).toBe(false);
		expect(m.mailboxChanged).toBe(false);
		expect(m.passwordRequired).toBe(false);
		expect(m.loginIsAddress).toBe(true);
		expect(m.effectiveAddress).toBe('dmarc@example.com');
		expect(m.problem).toBeNull();
		expect(m.canTest).toBe(true);
		m.draft.username = 'other@example.com';
		await settle();
		expect(m.dirty).toBe(true);
		expect(m.mailboxChanged).toBe(true);
		expect(m.passwordRequired).toBe(true);
		expect(m.problem).toBe('Enter the password.');
		expect(m.canTest).toBe(false);
		stop();
	});

	it('needs a password for a mailbox without one; before any value nothing can be tested', async () => {
		const blank = await setup(null);
		expect(blank.m.saved).toBeNull();
		expect(blank.m.canTest).toBe(false);
		blank.stop();
		const { m, stop } = await setup(server({ has_password: false }));
		expect(m.passwordRequired).toBe(true);
		expect(m.problem).toBe('Enter the password.');
		stop();
	});

	it('test() uses the stored password for the saved mailbox and sends typed credentials otherwise', async () => {
		const { m, stop } = await setup(server());
		connectImap.mockResolvedValue(probe());
		const res = await m.checkLogin('INBOX');
		expect(connectImap).toHaveBeenLastCalledWith({ security: 'tls', folder: 'INBOX' });
		expect(res?.ok).toBe(true);
		expect(m.folders).toEqual([{ name: 'INBOX', delimiter: '/' }]);
		expect(m.folderStatus).toEqual({ name: 'INBOX', exists: true, messages: 3 });
		m.draft.password = 'secret';
		connectImap.mockResolvedValue(probe({ selected: { name: 'Reports', exists: true, messages: 0 } }));
		await m.checkLogin('Reports');
		expect(connectImap).toHaveBeenLastCalledWith({
			host: 'imap.example.com',
			port: 993,
			username: 'dmarc@example.com',
			password: 'secret',
			security: 'tls',
			folder: 'Reports'
		});
		// The examined folder is not the selected one.
		expect(m.folderStatus).toBeNull();
		stop();
	});

	it('a failed test becomes a failed probe; a later test supersedes an earlier one', async () => {
		const { m, stop } = await setup(server());
		connectImap.mockRejectedValueOnce(new Error('down'));
		await m.checkLogin('INBOX');
		expect(m.probe).toEqual({ ok: false, message: 'down', detail: null, folders: [], selected: null });
		let first!: (v: ConnectImapResponse) => void;
		connectImap.mockReturnValueOnce(new Promise((res) => (first = res)));
		connectImap.mockResolvedValueOnce(probe({ message: 'second' }));
		const p1 = m.checkLogin('A');
		const p2 = m.checkLogin('B');
		first(probe({ message: 'first' }));
		expect(await p1).toBeNull();
		expect((await p2)?.message).toBe('second');
		expect(m.probe?.message).toBe('second');
		expect(m.probing).toBe(false);
		stop();
	});

	it('onFolderChange() picks the folder, clears the probe and examines it', async () => {
		const { m, stop } = await setup(server());
		connectImap.mockResolvedValue(probe({ selected: { name: 'Reports', exists: true, messages: 1 } }));
		m.onFolderChange('Reports');
		expect(m.draft.folder).toBe('Reports');
		expect(m.probe).toBeNull();
		await settle();
		expect(connectImap).toHaveBeenLastCalledWith({ security: 'tls', folder: 'Reports' });
		expect(m.folderStatus?.messages).toBe(1);
		stop();
	});

	it('testConnection() logs in with the typed password, lists the folders and examines the saved folder; it saves nothing', async () => {
		const { m, stop } = await setup(server({ folder: 'Reports' }));
		connectImap.mockResolvedValue(
			probe({
				folders: [
					{ name: 'Drafts', delimiter: '/' },
					{ name: 'INBOX', delimiter: '/' },
					{ name: 'Reports', delimiter: '/' }
				],
				selected: { name: 'Reports', exists: true, messages: 12 }
			})
		);
		m.draft.password = 'secret';
		expect(m.loginChecked).toBe(false);
		await m.testConnection();
		expect(connectImap).toHaveBeenCalledWith({
			host: 'imap.example.com',
			port: 993,
			security: 'tls',
			username: 'dmarc@example.com',
			password: 'secret',
			folder: 'Reports'
		});
		expect(m.normalized.folder).toBe('Reports');
		expect(m.folders?.map((f) => f.name)).toEqual(['Drafts', 'INBOX', 'Reports']);
		expect(m.folderStatus).toEqual({ name: 'Reports', exists: true, messages: 12 });
		expect(m.loginChecked).toBe(true);
		// The password stays in the draft for the save.
		expect(m.normalized.password).toBe('secret');
		expect(saveMailbox).not.toHaveBeenCalled();
		expect(saveReporting).not.toHaveBeenCalled();
		// Another password is another login: it must be tested again.
		m.draft.password = 'other';
		expect(m.loginChecked).toBe(false);
		stop();
	});

	it('testConnection() on another mailbox starts from the default folder; a saved folder that is gone falls back to it', async () => {
		const { m, stop } = await setup(server({ folder: 'Reports' }));
		m.draft.host = 'mail.example.net';
		m.draft.password = 'pw';
		connectImap.mockResolvedValue(probe());
		await m.testConnection();
		expect(connectImap).toHaveBeenLastCalledWith(
			expect.objectContaining({ host: 'mail.example.net', folder: 'INBOX' })
		);
		expect(m.normalized.folder).toBe('INBOX');

		const other = await setup(server({ folder: 'Old' }));
		connectImap.mockResolvedValueOnce(probe({ selected: { name: 'Old', exists: false, messages: null } }));
		connectImap.mockResolvedValueOnce(probe());
		await other.m.testConnection();
		expect(connectImap).toHaveBeenLastCalledWith({ security: 'tls', folder: 'INBOX' });
		expect(other.m.draft.folder).toBe('INBOX');
		expect(other.m.folderStatus?.exists).toBe(true);
		other.stop();
		stop();
	});

	it('testConnection() that is refused lists nothing and leaves the login unchecked', async () => {
		const { m, stop } = await setup(server());
		connectImap.mockResolvedValue(
			probe({ ok: false, message: 'Login failed', detail: 'AUTHENTICATE failed', folders: [] })
		);
		const res = await m.testConnection();
		expect(res).toMatchObject({ ok: false, message: 'Login failed', detail: 'AUTHENTICATE failed' });
		expect(m.folders).toBeNull();
		expect(m.loginChecked).toBe(false);
		stop();
	});

	it('saveConnection() saves connection, folder and report address together, only after a test of that login', async () => {
		const { m, stop } = await setup(
			server({ configured: false, has_password: false, host: '', username: '', report_address: '' })
		);
		m.draft.host = 'imap.example.com';
		m.draft.username = 'dmarc@example.com';
		m.draft.password = 'secret';
		await expect(m.saveConnection(m.normalized, m.saved!)).rejects.toThrow('Test the connection first.');
		connectImap.mockResolvedValue(
			probe({
				folders: [
					{ name: 'INBOX', delimiter: '/' },
					{ name: 'Reports', delimiter: '/' }
				]
			})
		);
		await m.testConnection();
		connectImap.mockResolvedValue(probe({ selected: { name: 'Reports', exists: true, messages: 40 } }));
		m.onFolderChange('Reports');
		await settle();
		saveMailbox.mockResolvedValue(server({ folder: 'Reports' }));
		saveReporting.mockResolvedValue(server({ folder: 'Reports' }));
		await m.saveConnection(m.normalized, m.saved!);
		expect(saveMailbox).toHaveBeenCalledTimes(1);
		expect(saveMailbox).toHaveBeenCalledWith({
			host: 'imap.example.com',
			port: 993,
			security: 'tls',
			username: 'dmarc@example.com',
			password: 'secret',
			folder: 'Reports',
			poll_interval_secs: 600
		});
		expect(saveReporting).toHaveBeenCalledWith({ report_address: 'dmarc@example.com' });
		stop();
	});

	it('saveConnection() refuses a folder the server does not have', async () => {
		const { m, stop } = await setup(server());
		connectImap.mockResolvedValue(probe({ selected: { name: 'INBOX', exists: false, messages: null } }));
		await m.testConnection();
		await expect(m.saveConnection(m.normalized, m.saved!)).rejects.toThrow('Pick a folder the server has.');
		expect(saveMailbox).not.toHaveBeenCalled();
		stop();
	});

	it('connectionKey() covers host (any case), port, security, username and password', () => {
		const d = { ...BLANK, host: 'IMAP.example.com', port: 993, username: 'u', password: 'p' };
		expect(connectionKey(d)).toBe(
			connectionKey({ ...d, host: 'imap.example.com', folder: 'Other', mode: 'forwarded' })
		);
		expect(connectionKey(d)).not.toBe(connectionKey({ ...d, password: '' }));
		expect(connectionKey(d)).not.toBe(connectionKey({ ...d, security: 'starttls' }));
	});

	it('saveDelivery() saves the reporting mode only', async () => {
		const { m, stop } = await setup(server());
		saveReporting.mockResolvedValue(server({ reporting_mode: 'forwarded' }));
		m.draft.mode = 'forwarded';
		const saved = await m.saveDelivery(m.normalized);
		expect(saveMailbox).not.toHaveBeenCalled();
		expect(saveReporting).toHaveBeenCalledWith({ reporting_mode: 'forwarded' });
		expect(saved.reporting.reporting_mode).toBe('forwarded');
		stop();
	});

	it('saveSettings() saves without a login when only the schedule changed: mailbox, reporting, then monitoring', async () => {
		const { m, stop } = await setup(server());
		saveMailbox.mockResolvedValue(server());
		saveReporting.mockResolvedValue(server());
		saveMonitoring.mockResolvedValue(server({ report_freshness_warning_hours: 24 }));
		m.draft.pollMinutes = 5;
		m.draft.warnHours = 24;
		m.draft.stoppedDays = 10;
		const result = await m.saveSettings(m.normalized, m.saved!);
		expect(connectImap).not.toHaveBeenCalled();
		expect(saveMailbox).toHaveBeenCalledWith({
			host: 'imap.example.com',
			port: 993,
			security: 'tls',
			username: 'dmarc@example.com',
			password: undefined,
			folder: 'INBOX',
			poll_interval_secs: 300
		});
		expect(saveReporting).toHaveBeenCalledWith({ report_address: 'dmarc@example.com', reporting_mode: 'direct' });
		expect(saveMonitoring).toHaveBeenCalledWith({
			report_freshness_warning_hours: 24,
			domain_reports_stopped_days: 10
		});
		expect(result).toEqual({ saved: server({ report_freshness_warning_hours: 24 }), message: 'Saved' });
		stop();
	});

	it('saveSettings() leaves the monitoring group alone when the arrival warning did not change', async () => {
		const { m, stop } = await setup(server());
		saveMailbox.mockResolvedValue(server());
		saveReporting.mockResolvedValue(server({ reporting_mode: 'forwarded' }));
		m.draft.mode = 'forwarded';
		const result = await m.saveSettings(m.normalized, m.saved!);
		expect(saveMonitoring).not.toHaveBeenCalled();
		expect(result.saved.reporting.reporting_mode).toBe('forwarded');
		stop();
	});

	it('saveSettings() logs in first for a changed folder and refuses one the server does not have', async () => {
		const { m, stop } = await setup(server());
		m.draft.folder = 'Nope';
		connectImap.mockResolvedValue(probe({ selected: { name: 'Nope', exists: false, messages: null } }));
		await expect(m.saveSettings(m.normalized, m.saved!)).rejects.toThrow('Folder "Nope" was not found on the server.');
		expect(saveMailbox).not.toHaveBeenCalled();
		connectImap.mockResolvedValue(probe({ selected: { name: 'Nope', exists: true, messages: 0 } }));
		saveMailbox.mockResolvedValue(server({ folder: 'Nope' }));
		saveReporting.mockResolvedValue(server({ folder: 'Nope' }));
		const result = await m.saveSettings(m.normalized, m.saved!);
		expect(connectImap).toHaveBeenLastCalledWith({ security: 'tls', folder: 'Nope' });
		expect(saveMailbox).toHaveBeenCalledWith(expect.objectContaining({ folder: 'Nope', poll_interval_secs: 600 }));
		expect(saveMonitoring).not.toHaveBeenCalled();
		expect(result.message).toBe('Connected and saved');
		stop();
	});

	it('setSecurity() moves an untouched port to the usual one and keeps a typed one', async () => {
		const { m, stop } = await setup(server());
		m.setSecurity('starttls');
		expect(m.draft).toMatchObject({ security: 'starttls', port: 143 });
		expect(m.defaultPort).toBe(143);
		m.setSecurity('tls');
		expect(m.draft).toMatchObject({ security: 'tls', port: 993 });
		m.draft.port = 1993;
		m.setSecurity('starttls');
		expect(m.draft).toMatchObject({ security: 'starttls', port: 1993 });
		await settle();
		expect(m.dirty).toBe(true);
		stop();
	});

	it('saveSettings() logs in first when only the security changed, and sends it', async () => {
		const { m, stop } = await setup(server());
		m.draft.password = 'secret';
		m.setSecurity('starttls');
		connectImap.mockResolvedValue(probe());
		saveMailbox.mockResolvedValue(server({ security: 'starttls', port: 143 }));
		saveReporting.mockResolvedValue(server({ security: 'starttls', port: 143 }));
		const result = await m.saveSettings(m.normalized, m.saved!);
		expect(connectImap).toHaveBeenLastCalledWith(
			expect.objectContaining({ host: 'imap.example.com', port: 143, security: 'starttls', password: 'secret' })
		);
		expect(saveMailbox).toHaveBeenCalledWith(expect.objectContaining({ port: 143, security: 'starttls' }));
		expect(result.message).toBe('Connected and saved');
		stop();
	});

	it('saveSettings() refuses an incomplete draft before any request', async () => {
		const { m, stop } = await setup(server());
		m.draft.host = '';
		await expect(m.saveSettings(m.normalized, m.saved!)).rejects.toThrow('Enter the server host.');
		expect(connectImap).not.toHaveBeenCalled();
		expect(saveMailbox).not.toHaveBeenCalled();
		stop();
	});
});
