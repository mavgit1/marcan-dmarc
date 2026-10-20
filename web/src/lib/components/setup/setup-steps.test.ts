import { describe, expect, it } from 'vitest';
import { CONNECTED, settingsFixture, type SettingsOverrides } from '$lib/test/settings-fixture';
import type { ReportingDomain } from '$lib/types/ReportingDomain';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import type { ScopeDomain } from '$lib/types/ScopeDomain';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { firstOpenStep, SETUP_STEPS, type SetupData, type StepId } from './setup-steps';

function data(over: Partial<SetupData> = {}, ...settings: SettingsOverrides[]): SetupData {
	return {
		organization: { id: 1, name: 'Default' } as SetupData['organization'],
		domains: [],
		settings: settingsFixture({ onboarding: { state: null } }, ...settings),
		health: { imap_progress: null, last_poll: { at: null, failed: false } } as SystemHealthResponse,
		reporting: null,
		organizationConfirmed: false,
		...over
	};
}

const done = (d: SetupData): StepId[] => SETUP_STEPS.filter((s) => s.done(d)).map((s) => s.id);
const domain: ScopeDomain = { id: 1, domain: 'example.com', organization_id: 1, has_reports: true };
const row = (set_up: boolean) => ({ id: 1, set_up }) as ReportingDomain;
const reporting = (rows: ReportingDomain[]) => ({ domains: rows }) as ReportingSetupResponse;

describe('setup steps', () => {
	it('a fresh installation opens on the organization', () => {
		expect(done(data())).toEqual([]);
		expect(firstOpenStep(data())).toBe(0);
	});

	it('a confirmed organization opens the mailbox', () => {
		expect(SETUP_STEPS[firstOpenStep(data({ organizationConfirmed: true }))].id).toBe('mailbox');
	});

	it('the import is done once the last pass succeeded and none runs', () => {
		const health = (running: boolean, at: number | null, failed = false) =>
			({ imap_progress: running ? {} : null, last_poll: { at, failed } }) as SystemHealthResponse;
		const connected = { reporting: { report_address: 'dmarc@example.net' } };
		expect(done(data({ health: health(true, 5) }, CONNECTED, connected))).not.toContain('import');
		expect(done(data({ health: health(false, null) }, CONNECTED, connected))).not.toContain('import');
		expect(done(data({ health: health(false, 5, true) }, CONNECTED, connected))).not.toContain('import');
		expect(done(data({ health: health(false, 5) }, CONNECTED, connected))).toContain('import');
	});

	it('the DMARC records are done when the server says every domain is set up', () => {
		expect(done(data({ domains: [domain], reporting: reporting([row(true), row(false)]) }))).not.toContain('dmarc');
		expect(done(data({ domains: [domain], reporting: reporting([]) }))).not.toContain('dmarc');
		expect(done(data({ domains: [domain], reporting: reporting([row(true)]) }))).toContain('dmarc');
	});

	it('everything done but the finish opens the last step', () => {
		const d = data(
			{
				domains: [domain],
				reporting: reporting([row(true)]),
				health: { imap_progress: null, last_poll: { at: 5, failed: false } } as SystemHealthResponse
			},
			CONNECTED,
			{ reporting: { report_address: 'dmarc@example.net', reporting_mode_saved: true } }
		);
		expect(done(d)).toEqual(['organization', 'mailbox', 'import', 'domains', 'delivery', 'dmarc']);
		expect(SETUP_STEPS[firstOpenStep(d)].id).toBe('done');
	});

	it('a finished setup ticks Done only once every step before it is done', () => {
		const finished = { onboarding: { state: 'completed' as const } };
		expect(done(data({}, finished))).not.toContain('done');
		const d = data(
			{
				domains: [domain],
				reporting: reporting([row(true)]),
				health: { imap_progress: null, last_poll: { at: 5, failed: false } } as SystemHealthResponse
			},
			CONNECTED,
			{ reporting: { report_address: 'dmarc@example.net', reporting_mode_saved: true } },
			finished
		);
		expect(done(d)).toContain('done');
	});
});
