import type { DomainOverview } from '$lib/types/DomainOverview';
import type { FailureOutcome } from '$lib/types/FailureOutcome';
import type { Outcome } from '$lib/types/Outcome';
import { formatInt } from '$lib/format';
import { outcomeCounts, type MailFigures, type OutcomeCounts } from '$lib/health';
import { statusDot, type StatusTone } from '$lib/tokens';

/** The one colour of each outcome, as in every chart, tile and badge. */
export const OUTCOME_STATUS = {
	delivered: 'fail',
	forwarded: 'forward',
	blocked: 'info',
	passed: 'pass'
} as const satisfies Record<Outcome, StatusTone>;

/** A row's outcome badge. */
export const OUTCOME_LABEL = {
	delivered: 'Delivered failure',
	forwarded: 'Forwarded',
	blocked: 'Blocked',
	passed: 'DMARC pass'
} as const satisfies Record<Outcome, string>;

/**
 * The badge's title: how DMARC passed (`verdictTitle`, from the DMARC verdict) or which
 * disposition blocked it.
 */
export const OUTCOME_TITLE: Record<
	Outcome,
	(ctx: { disposition: string | null | undefined; verdictTitle: string }) => string
> = {
	delivered: () => 'Failed DMARC and was still delivered',
	forwarded: () => 'Failed DMARC in transit; delivered because the receiver trusted the forwarder',
	blocked: ({ disposition: d }) =>
		d === 'reject'
			? 'Failed DMARC and was rejected'
			: d === 'quarantine'
				? 'Failed DMARC and was quarantined'
				: `Failed DMARC and was not delivered (${d})`,
	passed: ({ verdictTitle }) => verdictTitle
};

/** A kind the Failures tab's Show list ticks; `all_failures` is the first three together. */
export type FailureKind = Exclude<FailureOutcome, 'all_failures'>;

/** The Show list's kinds, in order. */
export const FAILURE_KINDS = [
	'delivered',
	'forwarded',
	'blocked',
	'dkim_only',
	'spf_only'
] as const satisfies readonly FailureKind[];

/** What `?outcome=all_failures` means: every DMARC failure, forwarded included (the tab ticks every kind by default). */
export const DEFAULT_FAILURE_KINDS = ['delivered', 'forwarded', 'blocked'] as const satisfies readonly FailureKind[];

/** The Failures tab's Show kinds (also the export dialog's scope line). */
export const OUTCOME_FILTER_LABEL = {
	delivered: 'Delivered failures',
	forwarded: 'Forwarded',
	blocked: 'Blocked by policy',
	dkim_only: 'DMARC pass via DKIM only',
	spf_only: 'DMARC pass via SPF only'
} as const satisfies Record<FailureKind, string>;

export const OUTCOME_FILTER_TITLE = {
	delivered: 'Failed DMARC and still delivered, not a trusted forward',
	forwarded: 'Failed DMARC, delivered because the receiver trusted the forwarder',
	blocked: 'Failed DMARC and was quarantined or rejected',
	dkim_only: 'DMARC passed through DKIM; SPF did not pass for your domain',
	spf_only: 'DMARC passed through SPF; DKIM did not pass for your domain'
} as const satisfies Record<FailureKind, string>;

/** Messages of each Show kind, from the domain overview's counts for the page's range. */
export function failureKindCounts(overview: DomainOverview): Record<FailureKind, number> {
	return {
		delivered: overview.delivered_failures,
		forwarded: overview.forwarded_failures,
		blocked: overview.blocked_messages,
		dkim_only: overview.dkim_only_messages,
		spf_only: overview.spf_only_messages
	};
}

/**
 * The kinds an `outcome` set covers, in Show order (`all_failures` is the three failure kinds).
 * Empty or missing is all five.
 */
export function failureKinds(outcome: readonly FailureOutcome[] | null | undefined): FailureKind[] {
	if (!outcome?.length) return [...FAILURE_KINDS];
	const set = new Set<FailureOutcome>(outcome);
	if (set.has('all_failures')) for (const kind of DEFAULT_FAILURE_KINDS) set.add(kind);
	return FAILURE_KINDS.filter((kind) => set.has(kind));
}

/** The Show trigger: the kind's name when one is ticked, "All kinds" for all, else "3 kinds". */
export function failureKindsSummary(kinds: readonly FailureKind[]): string {
	if (kinds.length === 1) return OUTCOME_FILTER_LABEL[kinds[0]];
	if (kinds.length >= FAILURE_KINDS.length) return 'All kinds';
	return `${kinds.length} kinds`;
}

/** The export dialog's scope part: the ticked kinds by name; none when all are. */
export function failureKindsScope(outcome: readonly FailureOutcome[] | null | undefined): string | null {
	const kinds = failureKinds(outcome);
	if (kinds.length >= FAILURE_KINDS.length) return null;
	return kinds.map((kind) => OUTCOME_FILTER_LABEL[kind]).join(', ');
}

/** What each of the four outcomes means, for a legend cell's tooltip (the Show kinds' titles). */
export const OUTCOME_EXPLANATION = {
	compliant: 'Passed DMARC through SPF or DKIM for your domain',
	blocked: OUTCOME_FILTER_TITLE.blocked,
	forwarded: OUTCOME_FILTER_TITLE.forwarded,
	delivered: OUTCOME_FILTER_TITLE.delivered
} as const satisfies Record<keyof OutcomeCounts, string>;

/** The four outcomes as words after a count ("225 blocked"), for a line without swatches. */
export const OUTCOME_COUNT_WORD = {
	compliant: 'DMARC pass',
	blocked: 'blocked',
	forwarded: 'forwarded',
	delivered: 'delivered failures'
} as const satisfies Record<keyof OutcomeCounts, string>;

/** The four counts of `mail` in one line: "18,550 DMARC pass, 225 blocked, 192 forwarded, 0 delivered failures". */
export function outcomeCountLine(mail: MailFigures): string {
	const counts = outcomeCounts(mail);
	return OUTCOMES.map((o) => `${formatInt(counts[o.key])} ${OUTCOME_COUNT_WORD[o.key]}`).join(', ');
}

/**
 * The four outcomes of all mail, in the colours every chart, tile and badge uses. Keys
 * match `OutcomeCounts` and the `DailyPoint` series; `color` is the bg class, `cssVar`
 * the chart colour.
 */
export const OUTCOMES: readonly {
	key: keyof OutcomeCounts;
	label: string;
	tone: StatusTone;
	color: string;
	cssVar: string;
}[] = (
	[
		['compliant', 'DMARC pass', 'pass'],
		['blocked', 'Blocked by policy', 'info'],
		['forwarded', 'Forwarded', 'forward'],
		['delivered', 'Delivered failures', 'fail']
	] as const
).map(([key, label, tone]) => ({
	key,
	label,
	tone,
	color: statusDot[tone],
	cssVar: `var(--status-${tone})`
}));
