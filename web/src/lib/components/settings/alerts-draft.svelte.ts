import { saveAlerts, sendTestAlert } from '$lib/api';
import { looksLikeEmail } from '$lib/report-address';
import { settings } from '$lib/state/settings.svelte';
import { listTokens } from '$lib/text';
import type { AlertSettings } from '$lib/types/AlertSettings';
import type { DigestFrequency } from '$lib/types/DigestFrequency';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import type { Weekday } from '$lib/types/Weekday';
import {
	invalid,
	savedSendingBlocks,
	savedTestRecipient,
	SettingsForm,
	type Invalid,
	type SettingsFormSource,
	type Writes
} from './settings-form.svelte';

/** The editable fields of the alerts group: the group, its recipients one per line as typed (commas work too). */
export type AlertsDraft = Omit<AlertSettings, 'recipients'> & { recipients: string };

export const DIGEST_LABEL = {
	off: 'Off',
	daily: 'Daily',
	weekly: 'Weekly'
} as const satisfies Record<DigestFrequency, string>;

export const WEEKDAY_LABEL = {
	monday: 'Monday',
	tuesday: 'Tuesday',
	wednesday: 'Wednesday',
	thursday: 'Thursday',
	friday: 'Friday',
	saturday: 'Saturday',
	sunday: 'Sunday'
} as const satisfies Record<Weekday, string>;

/** The addresses typed, without blanks and repeats (ignoring case), in typed order. */
export function recipientList(text: string): string[] {
	const seen = new Set<string>();
	return listTokens(text).filter((r) => {
		const key = r.toLowerCase();
		if (seen.has(key)) return false;
		seen.add(key);
		return true;
	});
}

/** `muted` (catalogue ids) with `ids` mailed (`mailed`) or muted: sorted, each once. */
export function withMailed(muted: readonly string[], ids: readonly string[], mailed: boolean): string[] {
	const set = new Set(muted);
	for (const id of ids) {
		if (mailed) set.delete(id);
		else set.add(id);
	}
	return [...set].sort();
}

/** Why Save waits while alert emails are switched on without a recipient (the server says the same). */
export const ALERTS_NO_RECIPIENT = 'Add at least one recipient first.';

/** The request for a normalized draft. */
export function toRequest(d: AlertsDraft): AlertSettings {
	return { ...d, recipients: recipientList(d.recipients) };
}

/**
 * The Alerts page's model (the alerts group): its switch, the recipients, the muted kinds, its
 * save (through `settings.save`) and the test alert to the test recipient.
 */
export class AlertsModel extends SettingsForm<SettingsResponse, AlertsDraft> {
	static readonly writes: Writes = { page: 'alerts', groups: ['alerts'] };
	/** Where the test goes: the test recipient saved under Sending. */
	testRecipient: string = $derived.by(() => savedTestRecipient(this.saved));
	/**
	 * The first typed recipient that is not an address, said under the field while typing. It
	 * holds nothing: a save sends the list, and the server's refusal (a 422, "The recipient … is
	 * not a valid email address.") is what the save bar shows.
	 */
	badRecipient: string | null = $derived.by(
		() => recipientList(this.draft.recipients).find((r) => !looksLikeEmail(r)) ?? null
	);

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({
			...options.source,
			label: 'Alerts',
			blank: {
				enabled: false,
				recipients: '',
				digest: 'off',
				digest_hour: 7,
				digest_weekday: 'monday',
				base_url: '',
				muted_kinds: []
			},
			feature: {
				title: 'Send alerts',
				hint: 'Emails each change and the digest to the recipients.',
				off: 'Turn on alert emails to send a test.'
			}
		});
	}

	protected toDraft(s: SettingsResponse): AlertsDraft {
		const { alerts } = s;
		return {
			...alerts,
			recipients: alerts.recipients.join('\n'),
			muted_kinds: withMailed(alerts.muted_kinds, [], true)
		};
	}

	/** One address per line, the base URL trimmed without a trailing slash, the muted kinds sorted. */
	protected normalize(d: AlertsDraft): AlertsDraft {
		return {
			...d,
			recipients: recipientList(d.recipients).join('\n'),
			digest_hour: Number(d.digest_hour),
			base_url: d.base_url.trim().replace(/\/+$/, ''),
			muted_kinds: withMailed(d.muted_kinds, [], true)
		};
	}

	/**
	 * Switched on without a recipient. Client-side because it holds Save, so the request the
	 * server would refuse (a 409, the same words) is never sent. It marks the recipients.
	 */
	protected whyBlocked(d: AlertsDraft): Invalid | null {
		return invalid('recipients', d.enabled && !d.recipients ? ALERTS_NO_RECIPIENT : null);
	}

	protected whyNoTest(): string | null {
		return savedSendingBlocks(this.saved, 'test');
	}

	/** Whether the kind `id` (a catalogue entry) is emailed in the draft. */
	isMailed(id: string): boolean {
		return !this.draft.muted_kinds.includes(id);
	}

	/** Email (`mailed`) or leave out the kinds `ids`: one entry, or a group's "all" / "none". */
	setMailed(ids: readonly string[], mailed: boolean): void {
		this.draft.muted_kinds = withMailed(this.draft.muted_kinds, ids, mailed);
	}

	/** The whole group; the server checks it (and answers every setting): a value it refuses is its 422's sentence on the save bar. */
	protected async write(d: AlertsDraft): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveAlerts(toRequest(d))) };
	}

	/**
	 * Send made-up changes to the test recipient, through the saved sending account, with the
	 * base URL on screen; the result line says how it went. A base URL the server refuses is its
	 * answer to the test (a 422).
	 */
	sendTest(): Promise<void> {
		return this.runTest(() => sendTestAlert({ alerts: toRequest(this.normalized) }));
	}
}
