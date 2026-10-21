import { saveRetention } from '$lib/api';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { Meta } from '$lib/types/Meta';
import type { RetentionSettings } from '$lib/types/RetentionSettings';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { invalid, SettingsForm, type Invalid, type SettingsFormSource, type Writes } from './settings-form.svelte';

/** The months a report can be kept, fewest and most. */
export type MonthRange = { min: number; max: number };

/** The server's range (`GET /api/meta`); `null` until it has loaded. */
export function retentionRange(m: Meta | null | undefined): MonthRange | null {
	return m ? { min: m.report_retention_months_min, max: m.report_retention_months_max } : null;
}

/**
 * Why `months` cannot be saved, worded for the save bar; `null` when it can (and while `range` is
 * not known). Client-side because the server clamps a value out of `range` instead of refusing
 * it: shown beside the field, it holds Save so nothing else than what is typed is stored.
 */
export function retentionProblem(months: number, range: MonthRange | null): string | null {
	if (!range) return null;
	const { min, max } = range;
	const n = Number(months);
	if (!Number.isInteger(n) || n < min || n > max) return `Keep reports for ${min} to ${max} months.`;
	return null;
}

/** The Data page's Retention section: the retention group as it is sent, saved through `settings.save`. */
export class RetentionModel extends SettingsForm<SettingsResponse, RetentionSettings> {
	static readonly writes: Writes = { page: 'data', groups: ['retention'] };
	/** The months the server keeps reports for, from `meta`; `null` until it has loaded. */
	range: MonthRange | null = $derived.by(() => retentionRange(meta.current));

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'Retention', blank: { report_retention_months: 0 } });
	}

	protected toDraft(s: SettingsResponse): RetentionSettings {
		return { ...s.retention };
	}

	protected normalize(d: RetentionSettings): RetentionSettings {
		return { report_retention_months: Number(d.report_retention_months) };
	}

	protected check(d: RetentionSettings): Invalid | null {
		return invalid('report_retention_months', retentionProblem(d.report_retention_months, this.range));
	}

	protected async write(d: RetentionSettings): Promise<{ saved: SettingsResponse }> {
		return { saved: await settings.save(saveRetention(d)) };
	}
}
