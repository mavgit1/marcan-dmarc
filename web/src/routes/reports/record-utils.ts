import type { ReportRecord } from '$lib/types/ReportRecord';

/** DMARC failed: neither SPF nor DKIM passed for an aligned domain. */
export function isAuthFailure(rec: ReportRecord): boolean {
	return rec.verdict === 'fail';
}

/**
 * Worst outcome first (the server's `outcome_rank`: delivered failure, forwarded, blocked,
 * passed), then higher volume.
 */
export function sortRecordsForensics<T extends ReportRecord>(records: T[]): T[] {
	return [...records].sort((a, b) => a.outcome_rank - b.outcome_rank || b.count - a.count);
}
