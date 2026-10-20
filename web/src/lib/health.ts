/**
 * Words for the server's verdicts on mail: health, enforcement and rates. The verdicts
 * themselves (thresholds, states, colours) come with every response (`crates/dmarc-assess`);
 * the words are the tables below.
 *
 * Two axes, deliberately not collapsed into one label: health answers "is the delivered
 * mail authentic?", enforcement "if someone spoofs me, is anything stopping them?".
 */

import type { Enforcement } from '$lib/types/Enforcement';
import type { EnforcementState } from '$lib/types/EnforcementState';
import type { MailCounts } from '$lib/types/MailCounts';
import type { MailVerdicts } from '$lib/types/MailVerdicts';
import type { HealthState } from '$lib/types/HealthState';
import type { Rate } from '$lib/types/Rate';
import type { Trend } from '$lib/types/Trend';
import type { TrendDirection } from '$lib/types/TrendDirection';
import { formatPct } from '$lib/format';
import type { StatusTone } from '$lib/tokens';

/** Is the delivered mail authentic? The server's state, as a word. */
export const HEALTH_LABEL = {
	no_data: 'No data',
	all_blocked: 'All blocked',
	protected: 'Protected',
	warning: 'Warning',
	critical: 'Critical'
} as const satisfies Record<HealthState, string>;

/**
 * `p=reject`, `p=quarantine`, `p=none`; an unknown state shows the published value (or
 * "Unknown" when there is none).
 */
export const ENFORCEMENT_LABEL: Record<EnforcementState, (policy: string | null | undefined) => string> = {
	reject: () => 'p=reject',
	quarantine: () => 'p=quarantine',
	monitor: () => 'p=none',
	unknown: (policy) => policy || 'Unknown'
};

/** `p=reject`, `p=quarantine`, `p=none`, or the published value when it is not one of those. */
export function enforcementLabel(enforcement: Enforcement, policy: string | null | undefined): string {
	return ENFORCEMENT_LABEL[enforcement.state](policy);
}

/** What every trend is measured against, said in its tooltip rather than on the line. */
const TREND_COMPARED = 'Compared with the previous period';

/**
 * A trend against the previous window, worded from its absolute change (`shown`, already
 * formatted): the signed figure alone, the arrow after it the figure's direction, the colour
 * the server's `status`. What it is compared with is the tooltip (`title`).
 */
export const TREND_LABEL: Record<
	TrendDirection,
	(shown: string) => { label: string; direction: 'up' | 'down' | 'flat'; title: string }
> = {
	new: () => ({ label: 'New', direction: 'up', title: 'None in the previous period' }),
	flat: () => ({ label: formatPct(0), direction: 'flat', title: TREND_COMPARED }),
	up: (shown) => ({ label: `+${shown}`, direction: 'up', title: TREND_COMPARED }),
	down: (shown) => ({ label: `-${shown}`, direction: 'down', title: TREND_COMPARED })
};

/**
 * A trend as it is shown (`StatCard`, `OutcomeLegend`): the figure, the arrow after it, the
 * server's colour and the tooltip.
 */
export type StatTrend = {
	label: string;
	direction: 'up' | 'down' | 'flat';
	/** What the change is measured against ("Compared with the previous period"). */
	title: string;
	/** The server's verdict on the change, independent of direction (more failures: `fail`). */
	tone: StatusTone;
};

/**
 * The server's trend against the previous window, worded. One decimal everywhere; compliance
 * moves in percentage points, shown with `%` like the counts so the figures read alike.
 * `undefined` without a previous window.
 */
export function trendWords(trend: Trend | null | undefined): StatTrend | undefined {
	if (!trend) return undefined;
	return { ...TREND_LABEL[trend.direction](formatPct(Math.abs(trend.delta ?? 0))), tone: trend.status };
}

/** A rate's percentage, or "—" when there was nothing to measure (never 0% or 100%). */
export function formatRate(rate: Rate): string {
	return rate.pct == null ? '—' : formatPct(rate.pct);
}

/** A scope's message figures as every response names them; `blocked_messages` is the server's. */
export type MailFigures = MailCounts & Pick<MailVerdicts, 'blocked_messages'>;

/** The four outcome counts of `mail`, keyed like `OUTCOMES` (`$lib/wording/outcome`). */
export function outcomeCounts(mail: MailFigures): OutcomeCounts {
	return {
		compliant: mail.compliant_messages,
		blocked: mail.blocked_messages,
		forwarded: mail.forwarded_failures,
		delivered: mail.delivered_failures
	};
}

/** Message counts of one sender or group, by outcome (as in `OUTCOMES`). */
export type OutcomeCounts = {
	compliant: number;
	blocked: number;
	forwarded: number;
	delivered: number;
};
