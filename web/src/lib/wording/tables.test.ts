// The shared wording tables (outcome): the interpolating ones read right, and the
// string ones have a word for every state (`satisfies Record<Enum, …>` keeps them complete).
import { describe, expect, it } from 'vitest';
import {
	failureKinds,
	failureKindsScope,
	failureKindsSummary,
	OUTCOME_FILTER_LABEL,
	OUTCOME_FILTER_TITLE,
	OUTCOME_LABEL,
	OUTCOME_TITLE,
	OUTCOMES,
	outcomeCountLine
} from './outcome';

const nonEmpty = (table: Record<string, string>) => Object.values(table).every((word) => word.trim() !== '');

describe('outcome wording', () => {
	it('has a word for every outcome and filter', () => {
		expect(nonEmpty(OUTCOME_LABEL)).toBe(true);
		expect(nonEmpty(OUTCOME_FILTER_LABEL)).toBe(true);
		expect(nonEmpty(OUTCOME_FILTER_TITLE)).toBe(true);
	});

	it('names the disposition that blocked a message', () => {
		expect(OUTCOME_TITLE.blocked({ disposition: 'quarantine', verdictTitle: '' })).toContain('quarantined');
		expect(OUTCOME_TITLE.blocked({ disposition: 'reject', verdictTitle: '' })).toContain('rejected');
		expect(OUTCOME_TITLE.passed({ disposition: null, verdictTitle: 'via DKIM' })).toBe('via DKIM');
	});

	it('lists the four outcomes in chart order, blocked in blue', () => {
		expect(OUTCOMES.map((o) => o.key)).toEqual(['compliant', 'blocked', 'forwarded', 'delivered']);
		expect(OUTCOMES.find((o) => o.key === 'blocked')?.tone).toBe('info');
	});

	it('puts the four counts in one line, in chart order', () => {
		const mail = {
			total_messages: 420,
			compliant_messages: 400,
			blocked_messages: 15,
			forwarded_failures: 5,
			delivered_failures: 0
		};
		expect(outcomeCountLine(mail)).toBe('400 DMARC pass, 15 blocked, 5 forwarded, 0 delivered failures');
	});

	it('reads an outcome set as Show kinds, in Show order', () => {
		expect(failureKinds(null)).toHaveLength(5);
		expect(failureKinds(['all_failures'])).toEqual(['delivered', 'forwarded', 'blocked']);
		expect(failureKinds(['spf_only', 'blocked', 'blocked'])).toEqual(['blocked', 'spf_only']);
	});

	it('summarizes the ticked kinds on the trigger and in the export scope', () => {
		expect(failureKindsSummary(['delivered'])).toBe('Delivered failures');
		expect(failureKindsSummary(['delivered', 'forwarded', 'blocked'])).toBe('3 kinds');
		expect(failureKindsSummary(failureKinds(null))).toBe('All kinds');
		expect(failureKindsScope(['blocked', 'dkim_only'])).toBe('Blocked by policy, DMARC pass via DKIM only');
		expect(failureKindsScope(null)).toBeNull();
	});
});
