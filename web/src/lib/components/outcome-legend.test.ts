// The legend's layout decisions: which cells, a zero kept in place and muted, a trend only
// where the server sent one, the share and meaning in the title.
import { describe, expect, it } from 'vitest';
import type { MailFigures } from '$lib/health';
import type { Statistics } from '$lib/types/Statistics';
import type { Trend } from '$lib/types/Trend';
import { legendCells, legendTotal, statisticsTrends, type OutcomeTrends } from './outcome-legend';

const mail: MailFigures = {
	total_messages: 18_967,
	compliant_messages: 18_550,
	blocked_messages: 225,
	forwarded_failures: 192,
	delivered_failures: 0
};

const up: Trend = { direction: 'up', delta: 12.5, status: 'info' };

describe('legendCells', () => {
	it('always shows the four outcomes in order, with their counts', () => {
		const cells = legendCells(mail);
		expect(cells.map((c) => c.key)).toEqual(['compliant', 'blocked', 'forwarded', 'delivered']);
		expect(cells.map((c) => c.label)).toEqual(['DMARC pass', 'Blocked by policy', 'Forwarded', 'Delivered failures']);
		expect(cells.map((c) => c.count)).toEqual([(18_550).toLocaleString(), '225', '192', '0']);
	});

	it('keeps a zero count in place, muted', () => {
		const cells = legendCells(mail);
		expect(cells.map((c) => c.muted)).toEqual([false, false, false, true]);
		const empty = legendCells({
			...mail,
			total_messages: 0,
			compliant_messages: 0,
			blocked_messages: 0,
			forwarded_failures: 0
		});
		expect(empty).toHaveLength(4);
		expect(empty.every((c) => c.muted)).toBe(true);
	});

	it('puts the share and the meaning in the title, and no share without mail', () => {
		const blocked = legendCells(mail)[1];
		expect(blocked.title).toBe('1.2% of all mail. Failed DMARC and was quarantined or rejected.');
		const none = legendCells({
			...mail,
			total_messages: 0,
			compliant_messages: 0,
			blocked_messages: 0,
			forwarded_failures: 0
		});
		expect(none[0].title).toBe('Passed DMARC through SPF or DKIM for your domain.');
	});

	it('shows a trend only where the server sent one', () => {
		expect(legendCells(mail).every((c) => c.trend === undefined)).toBe(true);
		const trends: OutcomeTrends = { compliant: null, blocked: up, forwarded: null, delivered: null };
		const cells = legendCells(mail, trends);
		expect(cells[1].trend).toEqual({
			label: '+12.5%',
			direction: 'up',
			title: 'Compared with the previous period',
			tone: 'info'
		});
		expect(cells.filter((c) => c.trend).map((c) => c.key)).toEqual(['blocked']);
	});
});

describe('legendTotal', () => {
	it('is labelled a total, the range being the header filter', () => {
		expect(legendTotal(mail)).toEqual({ label: 'Messages total', count: (18_967).toLocaleString() });
		expect(legendTotal({ ...mail, total_messages: 1 })).toEqual({ label: 'Messages total', count: '1' });
	});

	it('names a window that is not the range', () => {
		expect(legendTotal(mail, 'in this report').label).toBe('Messages in this report');
	});
});

describe('statisticsTrends', () => {
	it('takes the four outcome trends from the statistics', () => {
		const stats = {
			compliant_messages_trend: null,
			blocked_messages_trend: up,
			forwarded_failures_trend: null,
			delivered_failures_trend: up,
			total_messages_trend: null
		} as unknown as Statistics;
		expect(statisticsTrends(stats)).toEqual({ compliant: null, blocked: up, forwarded: null, delivered: up });
	});
});
