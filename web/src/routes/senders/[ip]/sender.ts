/**
 * Words for one sender's row per domain (this page): the row's verdict, its SPF and DKIM
 * lines in counts that add up, and its recipients. The verdict and the colours are the
 * server's (`row.verdict`, `row.spf`, `row.dkim`); this only phrases them.
 */
import type { OtherDomainPass } from '$lib/types/OtherDomainPass';
import type { SenderCheck } from '$lib/types/SenderCheck';
import type { SenderDomainRow } from '$lib/types/SenderDomainRow';
import type { SenderVerdictKind } from '$lib/types/SenderVerdictKind';
import type { DkimCoverage } from '$lib/types/DkimCoverage';
import { formatInt, formatPct, plural } from '$lib/format';
import { outcomeCounts } from '$lib/health';
import type { StatusTone } from '$lib/tokens';

export type SenderVerdict = { tone: StatusTone; label: string };

/**
 * Missing DKIM coverage, from the server's `dkim_coverage`: "No DKIM" when it says none of the
 * passing mail was signed (or nothing was measured), else the unsigned share, the rest of its
 * `pct` ("5.0% without DKIM"), as the domain's issue badge reads.
 */
export function dkimCoverageLabel(coverage: DkimCoverage): string {
	if (coverage.state === 'none' || coverage.pct == null) return 'No DKIM';
	return `${formatPct(100 - coverage.pct)} without DKIM`;
}

/** The row's one verdict, decided by the server (`row.verdict.kind`), in words. */
export const SENDER_VERDICT_LABEL: Record<SenderVerdictKind, (row: SenderDomainRow) => string> = {
	delivered_failures: (row) => plural(outcomeCounts(row).delivered, 'delivered failure'),
	forwarded: (row) => `${formatInt(outcomeCounts(row).forwarded)} forwarded`,
	blocked: (row) => `${formatInt(outcomeCounts(row).blocked)} blocked`,
	mostly_passed: () => 'Mostly passed',
	all_passed: () => 'All passed',
	dkim_coverage: (row) => dkimCoverageLabel(row.dkim_coverage),
	no_messages: () => 'No messages'
};

export function rowVerdict(row: SenderDomainRow): SenderVerdict {
	return { tone: row.verdict.status, label: SENDER_VERDICT_LABEL[row.verdict.kind](row) };
}

/**
 * One check (SPF or DKIM) in counts that add up to the row's messages: how many passed
 * for this domain, how many passed only for another domain (and which), and the rest.
 * "Pass" when the server says every message passed for this domain; the colour is the
 * server's (`row.spf` / `row.dkim`).
 */
function checkLine(
	row: SenderDomainRow,
	check: SenderCheck,
	aligned: number,
	others: OtherDomainPass[],
	rest: { count: number; label: string }[]
): { text: string; tone: StatusTone } {
	if (check.state === 'all_passed') return { text: 'Pass', tone: check.status };
	const total = row.total_messages;
	const parts: string[] = [];
	if (aligned > 0) parts.push(`${formatInt(aligned)} of ${formatInt(total)} passed for ${row.domain}`);
	for (const o of others) {
		const where = o.helo ? `${o.domain} (server name)` : o.domain;
		parts.push(`${formatInt(o.messages)} passed only for ${where}`);
	}
	for (const r of rest) if (r.count > 0) parts.push(`${formatInt(r.count)} ${r.label}`);
	return { text: parts.length > 0 ? parts.join(' · ') : 'None', tone: check.status };
}

export function spfLine(row: SenderDomainRow) {
	const failed = Math.max(0, row.total_messages - row.spf_pass_messages - row.spf_other_only_messages);
	return checkLine(row, row.spf, row.spf_pass_messages, row.spf_other_passes, [{ count: failed, label: 'failed' }]);
}

export function dkimLine(row: SenderDomainRow) {
	const failed = Math.max(
		0,
		row.total_messages - row.dkim_pass_messages - row.dkim_other_only_messages - row.dkim_unsigned_messages
	);
	return checkLine(row, row.dkim, row.dkim_pass_messages, row.dkim_other_passes, [
		{ count: row.dkim_unsigned_messages, label: 'unsigned' },
		{ count: failed, label: 'failed' }
	]);
}

/** Recipient domains of a row, busiest first (domain totals, else summed per receiver). */
export function recipientsOf(row: SenderDomainRow): { target: string; count: number }[] {
	const map = new Map<string, number>();
	const source = row.targets.length > 0 ? [row.targets] : row.receivers.map((r) => r.targets);
	for (const list of source) {
		for (const t of list) if (t.target) map.set(t.target, (map.get(t.target) ?? 0) + t.count);
	}
	return [...map].map(([target, count]) => ({ target, count })).sort((a, b) => b.count - a.count);
}
