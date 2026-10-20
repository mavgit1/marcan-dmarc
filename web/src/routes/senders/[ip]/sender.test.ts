// A sender row's verdict and check lines, worded from the server's counts and verdicts.
import { describe, expect, it } from 'vitest';
import type { DkimCoverage } from '$lib/types/DkimCoverage';
import type { SenderDomainRow } from '$lib/types/SenderDomainRow';
import { dkimCoverageLabel, rowVerdict, SENDER_VERDICT_LABEL, spfLine } from './sender';

const row = {
	domain: 'example.net',
	total_messages: 97,
	compliant_messages: 90,
	blocked_messages: 4,
	forwarded_failures: 2,
	delivered_failures: 1,
	no_aligned_dkim_messages: 9,
	dkim_coverage: { state: 'partial', pct: 90, status: 'warn' },
	spf: { state: 'partial', status: 'warn' },
	spf_pass_messages: 80,
	spf_other_only_messages: 10,
	spf_other_passes: [{ domain: 'relay.example', helo: true, messages: 10 }],
	verdict: { kind: 'delivered_failures', status: 'fail' }
} as unknown as SenderDomainRow;

describe('SENDER_VERDICT_LABEL', () => {
	it('counts the outcome the server picked', () => {
		expect(SENDER_VERDICT_LABEL.delivered_failures(row)).toBe('1 delivered failure');
		expect(SENDER_VERDICT_LABEL.blocked(row)).toBe('4 blocked');
		expect(SENDER_VERDICT_LABEL.forwarded(row)).toBe('2 forwarded');
		expect(SENDER_VERDICT_LABEL.dkim_coverage(row)).toBe('10.0% without DKIM');
	});

	it('takes the colour from the server', () => {
		expect(rowVerdict(row)).toEqual({ tone: 'fail', label: '1 delivered failure' });
	});
});

describe('spfLine', () => {
	it('adds up to the row total: for this domain, for another, failed', () => {
		expect(spfLine(row)).toEqual({
			text: '80 of 97 passed for example.net · 10 passed only for relay.example (server name) · 7 failed',
			tone: 'warn'
		});
	});

	it('reads "Pass" when every message passed for this domain', () => {
		const passed = { ...row, spf: { state: 'all_passed', status: 'pass' } } as SenderDomainRow;
		expect(spfLine(passed)).toEqual({ text: 'Pass', tone: 'pass' });
	});
});

describe('dkimCoverageLabel', () => {
	const coverage = (state: DkimCoverage['state'], pct: number | null) =>
		({ state, pct, status: 'warn' }) as DkimCoverage;

	it('says "No DKIM" when none of the passing mail is signed', () => {
		expect(dkimCoverageLabel(coverage('none', 0))).toBe('No DKIM');
		expect(dkimCoverageLabel(coverage('unmeasured', null))).toBe('No DKIM');
	});

	it("gives the unsigned share otherwise, the rest of the server's coverage", () => {
		expect(dkimCoverageLabel(coverage('partial', 95))).toBe('5.0% without DKIM');
	});
});
