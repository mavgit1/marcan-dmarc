import { saveMonitoring, saveReceiving } from '$lib/api';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { invalid, SettingsForm, type Invalid, type SettingsFormSource, type Writes } from './settings-form.svelte';

/** The editable fields under the Accounts page's cards: the schedule and the arrival warnings. */
export type ReceivingDraft = {
	/** How often every receiving account is read. */
	pollMinutes: number;
	/** The report warning: hours without any report, across all accounts. */
	warnHours: number;
	/** The domain warning: days without a report for one domain. */
	stoppedDays: number;
};

export const WARN_HOURS = [4, 12, 24, 48, 72];

/** The domain warning's choices; a stored value outside them is shown too. */
export const STOPPED_DAYS = [2, 3, 5, 7, 10, 14, 30];

const BLANK: ReceivingDraft = { pollMinutes: 0, warnHours: 0, stoppedDays: 0 };

export function fromServer(s: SettingsResponse): ReceivingDraft {
	return {
		pollMinutes: Math.max(1, Math.round(s.receiving.poll_interval_secs / 60)),
		warnHours: s.monitoring.report_freshness_warning_hours,
		stoppedDays: s.monitoring.domain_reports_stopped_days
	};
}

/**
 * The Accounts page's form below the cards (the receiving and monitoring groups): how often the
 * accounts are read, and when a missing report warns. Each group is saved only when it changed,
 * through `settings.save`.
 */
export class ReceivingModel extends SettingsForm<SettingsResponse, ReceivingDraft> {
	static readonly writes: Writes = { page: 'accounts', groups: ['receiving', 'monitoring'] };

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'Schedule', blank: BLANK });
	}

	protected toDraft(s: SettingsResponse): ReceivingDraft {
		return fromServer(s);
	}

	protected normalize(d: ReceivingDraft): ReceivingDraft {
		return {
			...d,
			pollMinutes: Number(d.pollMinutes),
			warnHours: Number(d.warnHours),
			stoppedDays: Number(d.stoppedDays)
		};
	}

	/** Below one minute the field is not a whole number of minutes; the server clamps the rest. */
	protected check(d: ReceivingDraft): Invalid | null {
		return invalid(
			'pollMinutes',
			Number.isInteger(d.pollMinutes) && d.pollMinutes >= 1 ? null : 'Check the accounts at least every minute.'
		);
	}

	protected async write(d: ReceivingDraft, s: SettingsResponse): Promise<{ saved: SettingsResponse }> {
		const base = fromServer(s);
		let saved = s;
		if (d.pollMinutes !== base.pollMinutes)
			saved = await settings.save(saveReceiving({ poll_interval_secs: d.pollMinutes * 60 }));
		if (d.warnHours !== base.warnHours || d.stoppedDays !== base.stoppedDays) {
			saved = await settings.save(
				saveMonitoring({
					report_freshness_warning_hours: d.warnHours,
					domain_reports_stopped_days: d.stoppedDays
				})
			);
			// The guide quotes the domain warning from `meta`.
			meta.reload().catch(() => {});
		}
		return { saved };
	}
}
