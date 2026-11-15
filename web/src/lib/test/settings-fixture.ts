import type { AccountView } from '$lib/types/AccountView';
import type { SettingsResponse } from '$lib/types/SettingsResponse';

/**
 * The one `SettingsResponse` the tests build on (`GET /api/settings`), so a new server field is
 * added here once. `settingsFixture()` is a fresh installation: no account, nothing sent, every
 * feature off. Overrides replace fields per group, applied in order: `settingsFixture(CONNECTED,
 * { sending: { enabled: true } })`.
 */

/** Fields to replace, per group; a list (`accounts`) is replaced whole. */
export type SettingsOverrides = {
	[G in keyof SettingsResponse]?: SettingsResponse[G] extends readonly unknown[]
		? SettingsResponse[G]
		: Partial<SettingsResponse[G]>;
};

/** The presets as the server answers them. */
export const PRESETS: SettingsResponse['presets'] = [
	{
		provider: 'generic',
		label: 'IMAP/SMTP',
		hint: 'Any provider: enter its servers under Advanced',
		docs: 'accounts#imap-smtp',
		imap: { host: '', port: 993, security: 'tls' },
		smtp: { host: '', port: null, security: 'tls' },
		login: 'password'
	},
	{
		provider: 'google',
		label: 'Google',
		hint: 'Log in with an app password, not the Google password',
		docs: 'accounts#google',
		imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
		smtp: { host: 'smtp.gmail.com', port: 465, security: 'tls' },
		login: 'password'
	},
	{
		provider: 'microsoft365',
		label: 'Microsoft 365',
		hint: 'Exchange Online: set up the mailbox with the guide first',
		docs: 'accounts#microsoft-365',
		imap: null,
		smtp: null,
		login: 'microsoft'
	}
];

/** One stored account (`dmarc@example.com`, receiving and able to send); `over` replaces fields. */
export function accountFixture(over: Partial<AccountView> = {}): AccountView {
	return {
		id: 1,
		name: 'Reports',
		provider: 'generic',
		address: 'dmarc@example.com',
		login: { kind: 'password', username: 'dmarc@example.com', has_password: true },
		imap: { host: 'imap.example.com', port: 993, security: 'tls' },
		smtp: { host: 'smtp.example.com', port: 465, security: 'tls' },
		receiving: { enabled: true, folder: 'INBOX', forwarded_from: [] },
		ready_to_receive: true,
		report_address: true,
		sends: true,
		ready_to_send: true,
		credential: { condition: 'ok', status: 'pass', lock: null, lock_text: null, expires_at: null, expiry_text: null },
		shared_mailbox: null,
		...over
	};
}

function base(): SettingsResponse {
	return {
		accounts: [],
		presets: PRESETS,
		account_defaults: { imap: { tls: 993, starttls: 143 }, smtp: { tls: 465, starttls: 587 }, folder: 'INBOX' },
		receiving: {
			poll_interval_secs: 300,
			defaults: { poll_interval_secs: 300, min_poll_interval_secs: 10, max_poll_interval_secs: 86400 }
		},
		monitoring: { report_freshness_warning_hours: 24, domain_reports_stopped_days: 7 },
		retention: { report_retention_months: 12 },
		dns: { nameservers: [], defaults: ['1.1.1.1', '8.8.8.8'] },
		sending: {
			enabled: false,
			account_id: null,
			reply_to: '',
			test_recipient: '',
			effective_test_recipient: ''
		},
		alerts: {
			enabled: false,
			recipients: [],
			digest: 'off',
			digest_hour: 7,
			digest_weekday: 'monday',
			base_url: '',
			muted_kinds: []
		},
		client_reports: {
			enabled: false,
			day_of_month: 1,
			hour_utc: 7,
			reply_to: '',
			next_send_at: null,
			organizations_due: 0,
			last_sent_at: null,
			held_reason: null,
			undelivered: null
		},
		license: { has_key: false, state: 'no_key', licensed_to: null, key_hint: null },
		branding: { operator_name: '', has_logo: false, logo_type: null, logo_bytes: null },
		onboarding: { state: 'completed' },
		updates: { check: true }
	};
}

/** One account receiving at `dmarc@example.com`, which could also send (sending is off). */
export const CONNECTED: SettingsOverrides = {
	accounts: [accountFixture()],
	sending: { effective_test_recipient: 'dmarc@example.com' }
};

/** The connected account with sending switched on through it: tests and previews can go out. */
export const SENDING_ON: SettingsOverrides = {
	...CONNECTED,
	sending: { ...CONNECTED.sending, enabled: true, account_id: 1 }
};

/** A whole settings answer: a fresh installation with `overrides` applied in order. */
export function settingsFixture(...overrides: SettingsOverrides[]): SettingsResponse {
	const s = base() as Record<keyof SettingsResponse, object>;
	for (const over of overrides)
		for (const [group, fields] of Object.entries(over) as [keyof SettingsResponse, object][])
			s[group] = Array.isArray(fields) ? fields : { ...s[group], ...fields };
	return s as SettingsResponse;
}
