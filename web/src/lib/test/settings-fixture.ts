import type { SettingsResponse } from '$lib/types/SettingsResponse';

/**
 * The one `SettingsResponse` the tests build on (`GET /api/settings`), so a new server field is
 * added here once. `settingsFixture()` is a fresh installation: no mailbox, nothing sent, every
 * feature off. Overrides replace fields per group, applied in order: `settingsFixture(CONNECTED,
 * { sending: { enabled: true } })`.
 */

/** Fields to replace, per group. */
export type SettingsOverrides = { [G in keyof SettingsResponse]?: Partial<SettingsResponse[G]> };

const MAILBOX_DEFAULTS = { port: 993, starttls_port: 143, folder: 'INBOX', poll_interval_secs: 300 };
const SENDING_DEFAULTS = { port: 465, starttls_port: 587 };

function base(): SettingsResponse {
	return {
		mailbox: {
			host: '',
			port: 993,
			security: 'tls',
			username: '',
			folder: 'INBOX',
			poll_interval_secs: 300,
			has_password: false,
			configured: false,
			defaults: { ...MAILBOX_DEFAULTS }
		},
		reporting: { report_address: '', report_org_domain: '', reporting_mode: 'direct', reporting_mode_saved: false },
		monitoring: { report_freshness_warning_hours: 24, domain_reports_stopped_days: 7 },
		retention: { report_retention_months: 12 },
		dns: { nameservers: [], defaults: ['1.1.1.1', '8.8.8.8'] },
		sending: {
			enabled: false,
			host: '',
			port: null,
			security: 'tls',
			same_login_as_mailbox: true,
			username: '',
			has_password: false,
			sender: '',
			reply_to: '',
			effective_sender: '',
			test_recipient: '',
			effective_test_recipient: '',
			defaults: { ...SENDING_DEFAULTS }
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

/** A connected mailbox (`dmarc@example.com` at `imap.example.com`), whose login the sending account would use. */
export const CONNECTED: SettingsOverrides = {
	mailbox: { host: 'imap.example.com', username: 'dmarc@example.com', has_password: true, configured: true },
	reporting: {
		report_address: 'dmarc@example.com',
		report_org_domain: 'example.com',
		reporting_mode_saved: true
	},
	sending: { effective_sender: 'dmarc@example.com', effective_test_recipient: 'dmarc@example.com' }
};

/** The connected mailbox with sending switched on through the same login: tests and previews can go out. */
export const SENDING_ON: SettingsOverrides = {
	...CONNECTED,
	sending: { ...CONNECTED.sending, enabled: true, host: 'imap.example.com', port: 465 }
};

/** A whole settings answer: a fresh installation with `overrides` applied in order. */
export function settingsFixture(...overrides: SettingsOverrides[]): SettingsResponse {
	const s = base() as Record<keyof SettingsResponse, object>;
	for (const over of overrides)
		for (const [group, fields] of Object.entries(over) as [keyof SettingsResponse, object][])
			s[group] = { ...s[group], ...fields };
	return s as SettingsResponse;
}
