// The Retention section: the stored months as its draft, which months can be saved, and the save.
import { describe, expect, it, vi } from 'vitest';
import { settingsFixture } from '$lib/test/settings-fixture';
import * as api from '$lib/api';
import { RetentionModel, retentionProblem } from './retention-draft.svelte';

vi.mock('$lib/api', () => ({ getSettings: vi.fn(), getMeta: vi.fn(), saveRetention: vi.fn() }));

const settle = () => new Promise<void>((done) => setTimeout(done));

describe('retention', () => {
	it('reads the retention group, not the monitoring one, and saves the months as numbers', async () => {
		const s = settingsFixture({
			monitoring: { report_freshness_warning_hours: 48 },
			retention: { report_retention_months: 24 }
		});
		vi.mocked(api.saveRetention).mockResolvedValue(s);
		let m!: RetentionModel;
		const stop = $effect.root(() => {
			m = new RetentionModel({ source: { value: () => s } });
		});
		await settle();
		expect(m.draft).toEqual({ report_retention_months: 24 });
		m.draft.report_retention_months = '36' as unknown as number;
		expect(m.dirty).toBe(true);
		await m.persist();
		expect(api.saveRetention).toHaveBeenCalledWith({ report_retention_months: 36 });
		stop();
	});

	it("accepts whole months in the server's range, typed or not", () => {
		const range = { min: 1, max: 120 };
		for (const months of [1, 12, 120, '6' as unknown as number]) expect(retentionProblem(months, range)).toBeNull();
		for (const months of [0, 121, 1.5, NaN])
			expect(retentionProblem(months, range)).toBe('Keep reports for 1 to 120 months.');
		// Until the range is known, the server checks.
		expect(retentionProblem(0, null)).toBeNull();
	});
});
