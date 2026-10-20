import { saveSending, sendTestMail } from '$lib/api';
import { emailDomain, looksLikeEmail } from '$lib/report-address';
import { settings } from '$lib/state/settings.svelte';
import type { SaveSendingRequest } from '$lib/types/SaveSendingRequest';
import type { SendingGroup } from '$lib/types/SendingGroup';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import type { SmtpSecurity } from '$lib/types/SmtpSecurity';
import { defaultPort, portAfterSecurityChange, sameAccount } from './connection';
import { NO_TEST_RECIPIENT, SettingsForm, type SettingsFormSource, type Writes } from './settings-form.svelte';

/**
 * The editable fields of the sending group: the group as the server answers it, the port
 * starting at the usual one when none is stored (`null` while the field is empty), and the
 * password, typed to change it (empty keeps the stored one).
 */
export type SendingDraft = SendingGroup & { password: string };

/** Why Save waits while sending is switched on without a server (the server says the same). */
export const SENDING_NO_SERVER = 'Enter a server first.';

/** Shown next to the password once host, port or username differ from the saved account. */
export const ACCOUNT_CHANGED_HINT = 'Different account: enter its password.';

/** A port field's value as a number; `null` when it is empty (an input's empty value is `null` or ''). */
function portValue(port: unknown): number | null {
	if (port === null || port === undefined || port === '') return null;
	return Number(port);
}

/** The address mail goes out from: the typed sender, else (same login) the mailbox login when it is one. */
export function effectiveSender(d: SendingDraft, mailboxUsername: string): string {
	if (d.sender) return d.sender;
	return d.same_login_as_mailbox && looksLikeEmail(mailboxUsername) ? mailboxUsername : '';
}

/** What the draft needs from the rest of the settings. */
export type SendingContext = {
	/** The SMTP account's own password must be typed (none stored, or another account). */
	passwordRequired: boolean;
	/** The saved mailbox username. */
	mailboxUsername: string;
	/** The mailbox has a username and a password. */
	mailboxConfigured: boolean;
};

/**
 * Why a normalized draft cannot send a test, worded like the server's own checks (which also
 * refuse port 25 and the mailbox login on another provider's server). While sending is off,
 * only the values typed must be valid. Client-side only to hold the Test button with its reason,
 * so no test goes out with an account the server would refuse; a save sends the group and the
 * save bar shows the server's refusal (its 409 or 422 sentence), no copy of it.
 */
export function problemWith(d: SendingDraft, ctx: SendingContext): string | null {
	if (d.port !== null && !(Number.isInteger(d.port) && d.port >= 1 && d.port <= 65535))
		return 'The SMTP port must be between 1 and 65535.';
	if (d.sender && !looksLikeEmail(d.sender)) return `The sender address ${d.sender} is not a valid email address.`;
	if (d.reply_to && !looksLikeEmail(d.reply_to))
		return `The Reply-To address ${d.reply_to} is not a valid email address.`;
	if (d.test_recipient && !looksLikeEmail(d.test_recipient))
		return `The test recipient ${d.test_recipient} is not a valid email address.`;
	if (!d.enabled) return null;
	if (!d.host) return SENDING_NO_SERVER;
	if (d.port === null) return "Enter the SMTP server's port.";
	if (d.same_login_as_mailbox) {
		if (!ctx.mailboxConfigured)
			return 'The mailbox has no login yet. Set up the mailbox first, or turn off “Same login as the mailbox” and enter the SMTP login.';
	} else {
		if (!d.username) return 'Enter the SMTP username.';
		if (ctx.passwordRequired && !d.password.trim()) return 'Enter the SMTP password.';
	}
	if (!effectiveSender(d, ctx.mailboxUsername)) return 'Enter the sender address.';
	return null;
}

/** The request for a normalized draft: the whole group, the password only when typed. */
export function toRequest(d: SendingDraft): SaveSendingRequest {
	return { ...d, password: d.password || undefined };
}

/**
 * The Sending page's model (the sending group): its switch, when its own password is needed,
 * its save (through `settings.save`), the security choice that moves an untouched port, and the
 * test send of the account on screen.
 */
export class SendingModel extends SettingsForm<SettingsResponse, SendingDraft> {
	static readonly writes: Writes = { page: 'sending', groups: ['sending'] };
	/** The SMTP account's own username and password are asked for (hidden with the same login). */
	ownLogin: boolean = $derived.by(() => !this.normalized.same_login_as_mailbox);
	/** Host, port or username differ from the saved account, so its password does not apply. */
	accountChanged: boolean = $derived.by(() => !!this.base && !sameAccount(this.normalized, this.base));
	passwordRequired: boolean = $derived.by(
		() => this.ownLogin && (!this.saved?.sending.has_password || this.accountChanged)
	);
	/** The address the draft would send from (the sender field's placeholder). */
	effectiveSender: string = $derived.by(() => effectiveSender(this.normalized, this.saved?.mailbox.username ?? ''));
	/** The draft's test goes to its test recipient, else the sender; empty when neither. */
	testRecipient: string = $derived.by(() => this.normalized.test_recipient || this.effectiveSender);
	/**
	 * The domain of the saved sender address, whose SPF and DMARC are shown: only once it is
	 * saved, so typing does not run a DNS check per keystroke.
	 */
	senderDomain: string | null = $derived.by(() => emailDomain(this.saved?.sending.effective_sender ?? ''));

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({
			...options.source,
			label: 'Sending',
			blank: {
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
			},
			secret: 'password',
			feature: {
				title: 'Sending',
				hint: 'Alerts and client reports go out through this account.',
				off: 'Turn on sending to send a test.'
			}
		});
	}

	protected toDraft(s: SettingsResponse): SendingDraft {
		const { has_password, effective_sender, effective_test_recipient, defaults, ...group } = s.sending;
		return { ...group, port: group.port ?? defaultPort(group.security, defaults), password: '' };
	}

	/** Trimmed, the port a number or `null`. */
	protected normalize(d: SendingDraft): SendingDraft {
		return {
			...d,
			host: d.host.trim(),
			port: portValue(d.port),
			username: d.username.trim(),
			sender: d.sender.trim(),
			reply_to: d.reply_to.trim(),
			test_recipient: d.test_recipient.trim()
		};
	}

	/**
	 * Switched on without a server. Client-side because it holds Save, so the request the server
	 * would refuse (a 409, the same words) is never sent.
	 */
	protected whyBlocked(d: SendingDraft): string | null {
		return d.enabled && !d.host ? SENDING_NO_SERVER : null;
	}

	/** The test sends with the values on screen, as complete as sending needs them, before they are saved. */
	protected whyNoTest(d: SendingDraft): string | null {
		const problem = problemWith(d, {
			passwordRequired: this.passwordRequired,
			mailboxUsername: this.saved?.mailbox.username ?? '',
			mailboxConfigured: !!this.saved?.mailbox.username && !!this.saved?.mailbox.has_password
		});
		return problem ?? (this.testRecipient ? null : NO_TEST_RECIPIENT);
	}

	/** The usual port for the draft's security, for the port field's placeholder. */
	get defaultPort(): number {
		return this.saved ? defaultPort(this.draft.security, this.saved.sending.defaults) : 0;
	}

	/** Another security picked: an empty or untouched port follows; a typed one stays. */
	setSecurity(security: SmtpSecurity) {
		if (this.saved)
			this.draft.port = portAfterSecurityChange(
				this.draft.port,
				this.draft.security,
				security,
				this.saved.sending.defaults
			);
		this.draft.security = security;
	}

	/** The whole group; the server checks it (and answers every setting): what it refuses is its sentence on the save bar. */
	protected async write(d: SendingDraft): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveSending(toRequest(d))) };
	}

	/** Send the test message with the account on screen to its test recipient; the result line says how it went. */
	sendTest(): Promise<void> {
		return this.runTest(() => sendTestMail({ sending: toRequest(this.normalized) }));
	}
}
