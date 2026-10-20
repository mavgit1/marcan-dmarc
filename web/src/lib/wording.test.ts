// The UI words the server's verdicts; these pin the words to the states they stand for.
import { describe, expect, it } from 'vitest';
import type { Enforcement } from '$lib/types/Enforcement';
import type { Rate } from '$lib/types/Rate';
import { enforcementLabel, formatRate, TREND_LABEL, trendWords } from './health';

const enforcement = (state: Enforcement['state']) => ({ state }) as Enforcement;

describe('enforcementLabel', () => {
	it('names the three policies', () => {
		expect(enforcementLabel(enforcement('reject'), 'reject')).toBe('p=reject');
		expect(enforcementLabel(enforcement('quarantine'), 'quarantine')).toBe('p=quarantine');
		expect(enforcementLabel(enforcement('monitor'), 'none')).toBe('p=none');
	});

	it('shows an unknown policy as published', () => {
		expect(enforcementLabel(enforcement('unknown'), 'bogus')).toBe('bogus');
		expect(enforcementLabel(enforcement('unknown'), null)).toBe('Unknown');
	});
});

describe('formatRate', () => {
	it('never shows an unmeasured rate as a number', () => {
		expect(formatRate({ pct: null } as Rate)).toBe('—');
		expect(formatRate({ pct: 100 } as Rate)).toBe('100.0%');
	});
});

describe('TREND_LABEL', () => {
	it('signs the change, the comparison in the tooltip only', () => {
		expect(TREND_LABEL.up('2.5%')).toEqual({
			label: '+2.5%',
			direction: 'up',
			title: 'Compared with the previous period'
		});
		expect(TREND_LABEL.down('2.5%').label).toBe('-2.5%');
		expect(TREND_LABEL.flat('0.0%')).toMatchObject({ label: '0.0%', direction: 'flat' });
	});

	it('names a figure without a predecessor', () => {
		expect(TREND_LABEL.new('')).toEqual({ label: 'New', direction: 'up', title: 'None in the previous period' });
	});
});

describe('trendWords', () => {
	it('words the server trend with one decimal and its status as the tone', () => {
		expect(trendWords({ direction: 'up', delta: 42.37, status: 'info' })).toEqual({
			label: '+42.4%',
			direction: 'up',
			title: 'Compared with the previous period',
			tone: 'info'
		});
		expect(trendWords({ direction: 'down', delta: -0.94, status: 'fail' })).toMatchObject({
			label: '-0.9%',
			tone: 'fail'
		});
	});

	it('is nothing without a previous window', () => {
		expect(trendWords(null)).toBeUndefined();
		expect(trendWords(undefined)).toBeUndefined();
	});
});
