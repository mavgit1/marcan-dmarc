import { describe, expect, it } from 'vitest';
import { CONNECTED, settingsFixture, type SettingsOverrides } from '$lib/test/settings-fixture';
import type { AccountHealth } from '$lib/types/AccountHealth';
import type { AddressAuthorization } from '$lib/types/AddressAuthorization';
import type { ReportingDomain } from '$lib/types/ReportingDomain';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import type { ScopeDomain } from '$lib/types/ScopeDomain';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { firstOpenStep, linkedStep, SETUP_STEPS, type SetupData, type StepId } from './setup-steps';

const account = (over: Partial<AccountHealth> = {}) =>
	({ id: 1, last_success_at: 5, last_error: null, ...over }) as AccountHealth;

const health = (running: boolean, accounts: AccountHealth[] = []) =>
	({ pass_progress: running ? { account_id: 1 } : null, accounts }) as SystemHealthResponse;

function data(over: Partial<SetupData> = {}, ...settings: SettingsOverrides[]): SetupData {
	return {
		organization: { id: 1, name: 'Default' } as SetupData['organization'],
		domains: [],
		settings: settingsFixture({ onboarding: { state: null } }, ...settings),
		health: health(false),
		reporting: null,
		organizationConfirmed: false,
		...over
	};
}

const done = (d: SetupData): StepId[] => SETUP_STEPS.filter((s) => s.done(d)).map((s) => s.id);
const domain: ScopeDomain = { id: 1, domain: 'example.com', organization_id: 1, has_reports: true };
const row = (set_up: boolean) => ({ id: 1, set_up }) as ReportingDomain;
const auth = (state: AddressAuthorization['state'], status: AddressAuthorization['status'] = 'neutral') =>
	({ domain: 'example.com', state, status }) as AddressAuthorization;
const reporting = (rows: ReportingDomain[], authorizations: AddressAuthorization[] = [auth('not_needed')]) =>
	({
		addresses: [{ account_id: 1, name: 'Reports', address: 'dmarc@example.com', forwarded_from: [] }],
		default_address: 'dmarc@example.com',
		authorizations,
		domains: rows
	}) as ReportingSetupResponse;

describe('setup steps', () => {
	it('come in their one order', () => {
		expect(SETUP_STEPS.map((s) => s.id)).toEqual([
			'organization',
			'accounts',
			'import',
			'domains',
			'addresses',
			'dmarc',
			'done'
		]);
	});

	it('a fresh installation opens on the organization', () => {
		expect(done(data())).toEqual([]);
		expect(firstOpenStep(data())).toBe(0);
	});

	it('a confirmed organization opens the accounts', () => {
		expect(SETUP_STEPS[firstOpenStep(data({ organizationConfirmed: true }))].id).toBe('accounts');
	});

	it('a link names a step, the ids before 0.4 their successors', () => {
		const id = (param: string | null) => SETUP_STEPS[linkedStep(param)]?.id ?? null;
		expect(id('dmarc')).toBe('dmarc');
		expect(id('mailbox')).toBe('accounts');
		expect(id('delivery')).toBe('addresses');
		expect(linkedStep('nonsense')).toBe(-1);
		expect(linkedStep(null)).toBe(-1);
	});

	it('the import is done once every account read without error and none runs', () => {
		expect(done(data({ health: health(true, [account()]) }, CONNECTED))).not.toContain('import');
		expect(done(data({ health: health(false, []) }, CONNECTED))).not.toContain('import');
		expect(done(data({ health: health(false, [account({ last_success_at: null })]) }, CONNECTED))).not.toContain(
			'import'
		);
		expect(
			done(data({ health: health(false, [account(), account({ id: 2, last_error: 'Login failed' })]) }, CONNECTED))
		).not.toContain('import');
		expect(done(data({ health: health(false, [account(), account({ id: 2 })]) }, CONNECTED))).toContain('import');
	});

	it('the report addresses are done once every address domain is authorized or needs nothing', () => {
		expect(done(data({ reporting: reporting([]) }))).toContain('addresses');
		expect(done(data({ reporting: reporting([], [auth('wildcard', 'pass'), auth('not_needed')]) }))).toContain(
			'addresses'
		);
		expect(done(data({ reporting: reporting([], [auth('no_wildcard')]) }))).not.toContain('addresses');
		expect(done(data({ reporting: reporting([], [auth('check_failed')]) }))).not.toContain('addresses');
		// No receiving account: no address yet.
		expect(done(data({ reporting: { ...reporting([]), addresses: [] } }))).not.toContain('addresses');
	});

	it('the DMARC records are done when the server says every domain is set up', () => {
		expect(done(data({ domains: [domain], reporting: reporting([row(true), row(false)]) }))).not.toContain('dmarc');
		expect(done(data({ domains: [domain], reporting: reporting([]) }))).not.toContain('dmarc');
		expect(done(data({ domains: [domain], reporting: reporting([row(true)]) }))).toContain('dmarc');
	});

	it('everything done but the finish opens the last step', () => {
		const d = data(
			{ domains: [domain], reporting: reporting([row(true)]), health: health(false, [account()]) },
			CONNECTED
		);
		expect(done(d)).toEqual(['organization', 'accounts', 'import', 'domains', 'addresses', 'dmarc']);
		expect(SETUP_STEPS[firstOpenStep(d)].id).toBe('done');
	});

	it('a finished setup ticks Done only once every step before it is done', () => {
		const finished = { onboarding: { state: 'completed' as const } };
		expect(done(data({}, finished))).not.toContain('done');
		const d = data(
			{ domains: [domain], reporting: reporting([row(true)]), health: health(false, [account()]) },
			CONNECTED,
			finished
		);
		expect(done(d)).toContain('done');
	});
});
