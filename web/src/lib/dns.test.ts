import { describe, expect, it } from 'vitest';
import { bimiStatus, mtaStsStatus, policyFetchLabel, spfErrorExplanation } from './dns';

const txt = { txt_lookup: 'ok', txt_record: 'v=STSv1; id=1', policy: undefined, mode_status: null } as const;

describe('mtaStsStatus', () => {
	it('shows the policy mode with the server verdict', () => {
		const v = mtaStsStatus({
			...txt,
			fetch: { kind: 'ok' },
			fetch_status: 'neutral',
			policy: { version: 'STSv1', mode: 'enforce', max_age: 86400, mx: ['mx.example.com'] },
			mode_status: 'pass'
		});
		expect(v).toEqual({ label: 'mode=enforce', tone: 'pass', mono: true });
	});

	it('calls an unreachable policy host out instead of blaming the check', () => {
		const v = mtaStsStatus({ ...txt, fetch: { kind: 'tls_failed', detail: 'x' }, fetch_status: 'warn' });
		expect(v).toEqual({ label: 'Policy unreachable', tone: 'warn' });
	});

	it('treats our own fetch timeout as a failed check', () => {
		expect(mtaStsStatus({ ...txt, fetch: { kind: 'timeout' }, fetch_status: 'neutral' }).label).toBe('Check failed');
	});

	it('reads the record alone when nothing was fetched', () => {
		expect(mtaStsStatus({ ...txt, fetch: { kind: 'not_attempted' }, fetch_status: 'neutral' }).label).toBe('Published');
		expect(
			mtaStsStatus({
				...txt,
				txt_record: undefined,
				txt_lookup: 'no_records',
				fetch: { kind: 'not_attempted' },
				fetch_status: 'neutral'
			}).label
		).toBe('No record');
	});
});

describe('bimiStatus', () => {
	it('names a declination', () => {
		const v = bimiStatus({
			records: ['v=BIMI1; l=; a=;'],
			lookup: 'ok',
			parsed: { version: 'BIMI1', logo_url: null, authority_url: null, declined: true }
		});
		expect(v).toEqual({ label: 'Declined', tone: 'neutral' });
	});

	it('otherwise reports presence', () => {
		expect(bimiStatus({ records: [], lookup: 'no_records', parsed: undefined }).label).toBe('No record');
	});
});

describe('policyFetchLabel', () => {
	it('words each outcome', () => {
		expect(policyFetchLabel({ kind: 'http_status', code: 404 })).toBe('HTTP 404');
		expect(policyFetchLabel({ kind: 'name_not_found' })).toBe('name not found');
		expect(policyFetchLabel({ kind: 'private_address', address: '10.0.0.5' })).toBe('private address 10.0.0.5');
	});
});

describe('spfErrorExplanation', () => {
	it('quotes the SERVFAIL detail and reads nothing into a missing error', () => {
		expect(spfErrorExplanation({ kind: 'servfail', detail: 'refused' })).toContain('(refused)');
		expect(spfErrorExplanation(undefined)).toBeNull();
	});
});
