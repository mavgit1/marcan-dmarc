import { describe, expect, it } from 'vitest';
import { formatCheckedAge, formatMegabytes, formatPct, plural, relativeTime } from './format';

describe('relativeTime', () => {
	const now = 1_800_000_000 * 1000;
	const ago = (secs: number) => relativeTime(now / 1000 - secs, now);

	it('rounds down into the largest unit', () => {
		expect(ago(0)).toBe('just now');
		expect(ago(59)).toBe('just now');
		expect(ago(60)).toBe('1m ago');
		expect(ago(3599)).toBe('59m ago');
		expect(ago(3600)).toBe('1h ago');
		expect(ago(86_399)).toBe('23h ago');
		expect(ago(86_400 * 3)).toBe('3d ago');
	});

	it('treats a timestamp in the future as now', () => {
		expect(ago(-120)).toBe('just now');
	});
});

describe('formatCheckedAge', () => {
	const now = 1_800_000_000 * 1000;

	it('accepts unix seconds and ISO strings', () => {
		expect(formatCheckedAge(1_800_000_000 - 7200, now)).toBe('checked 2h ago');
		expect(formatCheckedAge(new Date(now - 5 * 60_000).toISOString(), now)).toBe('checked 5m ago');
	});

	it('says when there is no or no usable time', () => {
		expect(formatCheckedAge(null, now)).toBe('not graded yet');
		expect(formatCheckedAge('', now)).toBe('not graded yet');
		expect(formatCheckedAge('not a date', now)).toBe('checked time unknown');
	});
});

describe('plural', () => {
	it('uses the singular for exactly one', () => {
		expect(plural(1, 'report')).toBe('1 report');
		expect(plural(0, 'report')).toBe('0 reports');
		expect(plural(2, 'report')).toBe('2 reports');
	});

	it('takes an irregular plural', () => {
		expect(plural(3, 'policy', 'policies')).toBe('3 policies');
	});
});

describe('number formats', () => {
	it('formats percentages and megabytes with one decimal', () => {
		expect(formatPct(99.04)).toBe('99.0%');
		expect(formatMegabytes(1.25 * 1024 * 1024)).toBe('1.3 MB');
	});
});
