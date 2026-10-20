import { saveClientReports, sendClientReportEmail } from '$lib/api';
import { looksLikeEmail } from '$lib/report-address';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { ClientReportSettings } from '$lib/types/ClientReportSettings';
import type { Meta } from '$lib/types/Meta';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import {
	savedSendingBlocks,
	savedTestRecipient,
	SettingsForm,
	type SettingsFormSource,
	type Writes
} from './settings-form.svelte';

/** Why Save waits while the reports are switched on with an empty list (the server says the same). */
export const CLIENT_REPORTS_NO_RECIPIENT = 'Add at least one recipient to the list first.';

/** The days a report can go out on, first and last. */
export type DayRange = { min: number; max: number };

/** The server's day range (`GET /api/meta`: every month has them); `null` until it has loaded. */
export function dayRange(m: Meta | null | undefined): DayRange | null {
	return m ? { min: m.client_report_day_min, max: m.client_report_day_max } : null;
}

/**
 * What is wrong with a normalized draft's day of the month, in `range` (`null` until `meta` has
 * loaded: nothing is checked here then, the server checks on save). Client-side because it is
 * shown beside the field while typing and holds Save, so no request is sent that the server would
 * refuse with the same words (a 422).
 */
export function dayProblem(d: ClientReportSettings, range: DayRange | null): string | null {
	if (!range) return null;
	if (!Number.isInteger(d.day_of_month) || d.day_of_month < range.min || d.day_of_month > range.max)
		return `The day of the month must be between ${range.min} and ${range.max}, so every month has it.`;
	return null;
}

/** What is wrong with a normalized draft's Reply-To, worded like the server's check. */
export function replyToProblem(d: ClientReportSettings): string | null {
	return d.reply_to && !looksLikeEmail(d.reply_to)
		? `The Reply-To address ${d.reply_to} is not a valid email address.`
		: null;
}

/**
 * The Client reports page's model (the client reports group): its switch, what is wrong with
 * it (which holds Save, and "Send a preview", which sends the Reply-To on screen; the server
 * refuses the same with the same words), its save (through `settings.save`) and "send a preview
 * to me" per organization (the test, to the test recipient saved under Sending, with the
 * Reply-To on screen).
 */
export class ClientReportsModel extends SettingsForm<SettingsResponse, ClientReportSettings> {
	static readonly writes: Writes = { page: 'client-reports', groups: ['client-reports'] };
	/** The organization the last preview was for. */
	previewing = $state<number | null>(null);
	/** Where the preview goes: the test recipient saved under Sending. */
	testRecipient: string = $derived.by(() => savedTestRecipient(this.saved));
	/** The day of the month's part of `problem`, shown beside that field. */
	dayProblem: string | null = $derived.by(() => dayProblem(this.normalized, dayRange(meta.current)));
	#recipientCount: () => number;

	constructor(options: {
		source: SettingsFormSource<SettingsResponse>;
		/** How many recipients the list has as it would be saved (the Recipients section's draft). */
		recipientCount: () => number;
	}) {
		super({
			...options.source,
			label: 'Schedule',
			blank: { enabled: false, day_of_month: 1, hour_utc: 7, reply_to: '' },
			feature: {
				title: 'Send client reports',
				hint: 'Emails each organization its report once a month.',
				off: 'Turn on client reports to send a preview.'
			}
		});
		this.#recipientCount = options.recipientCount;
	}

	protected toDraft(s: SettingsResponse): ClientReportSettings {
		const { enabled, day_of_month, hour_utc, reply_to } = s.client_reports;
		return { enabled, day_of_month, hour_utc, reply_to };
	}

	/** Numbers as numbers, the Reply-To trimmed. */
	protected normalize(d: ClientReportSettings): ClientReportSettings {
		return { ...d, day_of_month: Number(d.day_of_month), hour_utc: Number(d.hour_utc), reply_to: d.reply_to.trim() };
	}

	protected check(d: ClientReportSettings): string | null {
		return dayProblem(d, dayRange(meta.current)) ?? replyToProblem(d);
	}

	/** Switched on with an empty list. */
	protected whyBlocked(d: ClientReportSettings): string | null {
		return d.enabled && this.#recipientCount() === 0 ? CLIENT_REPORTS_NO_RECIPIENT : null;
	}

	/** The server asks only for sending, not for the reports to be on; the Reply-To on screen goes with it. */
	protected whyNoTest(d: ClientReportSettings): string | null {
		return savedSendingBlocks(this.saved, 'preview') ?? replyToProblem(d);
	}

	/**
	 * Switched on, saved after the recipients: the server needs the list saved first. Switched
	 * off, before them: the server keeps the list from being emptied while the reports are on.
	 */
	get saveLast(): boolean {
		return this.normalized.enabled;
	}

	/** The whole group; the server checks it again (and answers every setting). */
	protected async write(d: ClientReportSettings): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveClientReports(d)) };
	}

	/** Send `organizationId`'s report as it would go out now to the test recipient. */
	sendPreview(organizationId: number): Promise<void> {
		this.previewing = organizationId;
		return this.runTest(() =>
			sendClientReportEmail({ organization_id: organizationId, reply_to: this.normalized.reply_to })
		);
	}
}
