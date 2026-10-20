import type { Statistics } from '$lib/types/Statistics';
import type { Trend } from '$lib/types/Trend';
import { formatInt, formatPct, percent } from '$lib/format';
import { outcomeCounts, trendWords, type MailFigures, type OutcomeCounts, type StatTrend } from '$lib/health';
import { OUTCOME_EXPLANATION, OUTCOMES } from '$lib/wording/outcome';

/** The server's trend of each outcome count against the previous window (`Statistics`). */
export type OutcomeTrends = Record<keyof OutcomeCounts, Trend | null>;

/**
 * What a chart's legend is given (`VolumeChart`'s `outcomes`): the counts, and the two
 * choices a page has. `total` puts the total on the left, labelled like a stat card:
 * `true` for "Messages total" (the range is the header's filter), or the words for a window
 * that is not the range (a report's "in this report"); `trends` puts the change under each
 * count. `mail` `null` is the legend's skeleton.
 */
export type OutcomeFigures = {
	mail: MailFigures | null;
	total?: true | string;
	trends?: OutcomeTrends;
};

/** One cell of the legend: an outcome, its count and how it is drawn. */
export type LegendCell = {
	key: keyof OutcomeCounts;
	label: string;
	/** The swatch's background class (`OUTCOMES`). */
	swatch: string;
	count: string;
	/** A zero count: muted, its swatch faded, but in place. */
	muted: boolean;
	/** The share of all mail and what the outcome means. */
	title: string;
	trend?: StatTrend;
};

/** The four trends of `Statistics`, keyed like `OUTCOMES`. */
export function statisticsTrends(stats: Statistics): OutcomeTrends {
	return {
		compliant: stats.compliant_messages_trend,
		blocked: stats.blocked_messages_trend,
		forwarded: stats.forwarded_failures_trend,
		delivered: stats.delivered_failures_trend
	};
}

/**
 * The legend's four cells, always all four in `OUTCOMES` order, so the layout never shifts
 * and "Delivered failures 0" is shown. Nothing is proportional: the share is in the title.
 */
export function legendCells(mail: MailFigures, trends?: OutcomeTrends): LegendCell[] {
	const counts = outcomeCounts(mail);
	return OUTCOMES.map((o) => {
		const n = counts[o.key];
		const share = mail.total_messages > 0 ? `${formatPct(percent(n, mail.total_messages))} of all mail. ` : '';
		return {
			key: o.key,
			label: o.label,
			swatch: o.color,
			count: formatInt(n),
			muted: n === 0,
			title: `${share}${OUTCOME_EXPLANATION[o.key]}.`,
			trend: trends ? trendWords(trends[o.key]) : undefined
		};
	});
}

/**
 * The total on the legend's left, set apart from the cells: its label in the stat cards'
 * style ("Messages total", or "Messages" and `within` for a window that is not the header's
 * range: "Messages in this report") over the count.
 */
export function legendTotal(mail: MailFigures, within?: string): { label: string; count: string } {
	return {
		label: within ? `Messages ${within}` : 'Messages total',
		count: formatInt(mail.total_messages)
	};
}
