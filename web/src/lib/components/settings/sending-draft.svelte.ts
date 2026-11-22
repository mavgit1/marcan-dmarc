import { saveSending, sendTestMail } from '#lib/api.ts';
import { emailDomain, looksLikeEmail } from '#lib/report-address.ts';
import { settings } from '#lib/state/settings.svelte.ts';
import type { AccountView } from '#lib/types/AccountView.ts';
import type { SendingSettings } from '#lib/types/SendingSettings.ts';
import type { SettingsResponse } from '#lib/types/SettingsResponse.ts';
import {
	invalid,
	NO_TEST_RECIPIENT,
	SettingsForm,
	type Invalid,
	type SettingsFormSource,
	type Writes
} from './settings-form.svelte';

/** The editable fields of the sending group: the group as the server answers it. */
export type SendingDraft = SendingSettings;

/**
 * Why Save waits while sending is switched on without an account: the server's `NO_SENDING_ACCOUNT`
 * without "under Settings → Sending", since it is said on that page, at the account select.
 */
export const SENDING_NO_ACCOUNT = 'Choose an account to send through.';

/**
 * The accounts mail can go out through, as the server decides it (`sends`: an SMTP server, or a
 * Microsoft login, which sends over Microsoft Graph), in list order.
 */
export function sendingAccounts(s: SettingsResponse | null): AccountView[] {
	return s?.accounts.filter((a) => a.sends) ?? [];
}

/** The address mail goes out from: the chosen account's address, when it is one. */
export function senderOf(account: AccountView | undefined): string {
	return account && looksLikeEmail(account.address) ? account.address : '';
}

/** Why the chosen account cannot be sent from: the server's sentence for an account without an address. */
export function noSenderAddress(account: AccountView): string {
	return `${account.name} has no email address to send from. Enter one under Accounts.`;
}

/**
 * Why a normalized draft cannot send a test, worded like the server's own checks. Client-side
 * only to hold the Test button with its reason; a save sends the group and the save bar shows
 * the server's refusal (its 409 or 422 sentence), no copy of it.
 */
export function problemWith(d: SendingDraft, account: AccountView | undefined): string | null {
	if (d.reply_to && !looksLikeEmail(d.reply_to))
		return `The Reply-To address ${d.reply_to} is not a valid email address.`;
	if (d.test_recipient && !looksLikeEmail(d.test_recipient))
		return `The test recipient ${d.test_recipient} is not a valid email address.`;
	if (!account) return SENDING_NO_ACCOUNT;
	if (!senderOf(account)) return noSenderAddress(account);
	return null;
}

/**
 * The Sending page's model (the sending group): its switch, the account mail goes through and
 * from (one of the accounts that can send), Reply-To and test recipient, its save
 * (through `settings.save`) and the test send with the values on screen.
 */
export class SendingModel extends SettingsForm<SettingsResponse, SendingDraft> {
	static readonly writes: Writes = { page: 'sending', groups: ['sending'] };
	/** The accounts the select offers. */
	accounts: AccountView[] = $derived.by(() => sendingAccounts(this.saved));
	/** The account chosen on screen. */
	account: AccountView | undefined = $derived.by(() => this.accounts.find((a) => a.id === this.normalized.account_id));
	/** The address the draft would send from: the chosen account's; empty when it has none. */
	sender: string = $derived.by(() => senderOf(this.account));
	/** The draft's test goes to its test recipient, else the sender; empty when neither. */
	testRecipient: string = $derived.by(() => this.normalized.test_recipient || this.sender);
	/** The sender's domain, whose SPF and DMARC are shown. */
	senderDomain: string | null = $derived.by(() => emailDomain(this.sender));

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({
			...options.source,
			label: 'Sending',
			blank: { enabled: false, account_id: null, reply_to: '', test_recipient: '' },
			feature: {
				title: 'Send email',
				off: 'Turn on sending to send a test.'
			}
		});
	}

	protected toDraft(s: SettingsResponse): SendingDraft {
		const { enabled, account_id, reply_to, test_recipient } = s.sending;
		return { enabled, account_id, reply_to, test_recipient };
	}

	/** Trimmed; the select's value a number or `null`. */
	protected normalize(d: SendingDraft): SendingDraft {
		const id = d.account_id === null || (d.account_id as unknown) === '' ? null : Number(d.account_id);
		return {
			...d,
			account_id: id,
			reply_to: d.reply_to.trim(),
			test_recipient: d.test_recipient.trim()
		};
	}

	/**
	 * Switched on without an account. Client-side because it holds Save, so the request the server
	 * would refuse (a 409, the same words) is never sent. It marks the account select.
	 */
	protected whyBlocked(d: SendingDraft): Invalid | null {
		return invalid('account_id', d.enabled && d.account_id === null ? SENDING_NO_ACCOUNT : null);
	}

	/** The test sends with the values on screen, as complete as sending needs them, before they are saved. */
	protected whyNoTest(d: SendingDraft): string | null {
		return problemWith(d, this.account) ?? (this.testRecipient ? null : NO_TEST_RECIPIENT);
	}

	/** The whole group; the server checks it (and answers every setting): what it refuses is its sentence on the save bar. */
	protected async write(d: SendingDraft): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveSending(d)) };
	}

	/** Send the test message through the account on screen to its test recipient; the result line says how it went. */
	sendTest(): Promise<void> {
		return this.runTest(() => sendTestMail({ sending: this.normalized }));
	}
}
