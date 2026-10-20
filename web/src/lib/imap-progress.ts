import type { ImapPhase } from '$lib/types/ImapPhase';
import type { ImapProgressResponse } from '$lib/types/ImapProgressResponse';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { formatInt, percent, plural } from '$lib/format';

/**
 * Completion 0–100 of a running mailbox pass, by bytes (messages vary a lot in size),
 * falling back to message counts; `null` while the total is not known yet.
 */
export function progressPercent(p: ImapProgressResponse): number | null {
	if (p.total_bytes > 0) return Math.min(100, percent(p.processed_bytes, p.total_bytes));
	if (p.total_messages > 0) return Math.min(100, percent(p.processed_messages, p.total_messages));
	return null;
}

/** What a running mailbox pass is doing: "Importing 1,240 of 2,950 messages" and the shorter phases before it. */
export const IMAP_PHASE_LABEL: Record<ImapPhase, (p: ImapProgressResponse) => string> = {
	connecting: () => 'Connecting to the mailbox…',
	scanning: (p) =>
		p.total_messages > 0 ? `Found ${plural(p.total_messages, 'new message')}…` : 'Checking for new mail…',
	importing: (p) => `Importing ${formatInt(p.processed_messages)} of ${plural(p.total_messages, 'message')}`
};

/**
 * One line for a running pass outside the header: what it has stored so far
 * ("Importing the mailbox: 1,240 reports so far"), or the phase before that.
 */
export function importActivityLabel(p: ImapProgressResponse): string {
	if (p.phase !== 'importing') return IMAP_PHASE_LABEL[p.phase](p);
	return `Importing the mailbox: ${plural(p.reports_ingested, 'report')} so far (${formatInt(p.processed_messages)} of ${plural(p.total_messages, 'message')} read)`;
}

/**
 * The setup's import step in one line, from the health poll: "Importing: 1,204 reports from 14
 * domains so far, checking DNS" while a pass runs, "Imported: 1,204 reports from 14 domains"
 * once it ended; the DNS clause while new domains are being checked. `null` before any report.
 */
export function importSummary(health: SystemHealthResponse): string | null {
	const running = health.imap_progress != null;
	if (!running && health.total_reports === 0) return null;
	const counts = `${plural(health.total_reports, 'report')} from ${plural(health.dns_checks.domains, 'domain')}`;
	const dns = health.dns_checks.checking ? ', checking DNS' : '';
	return running ? `Importing: ${counts} so far${dns}` : `Imported: ${counts}${dns}`;
}

/** {@link IMAP_PHASE_LABEL} for the pass's current phase. */
export function progressLabel(p: ImapProgressResponse): string {
	return IMAP_PHASE_LABEL[p.phase](p);
}
