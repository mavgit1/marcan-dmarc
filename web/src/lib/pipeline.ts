/**
 * The ingestion pipeline's status from `GET /api/system/health`: is the mailbox being
 * polled, and are reports arriving? The header, Settings and the setup all read
 * it from here so they cannot disagree.
 */

import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { formatDateTime, relativeTime } from '$lib/format';
import { progressLabel } from '$lib/imap-progress';
import type { MailboxState } from '$lib/types/MailboxState';
import type { Pipeline } from '$lib/types/Pipeline';
import type { PipelineHeadline } from '$lib/types/PipelineHeadline';
import type { ReportsState } from '$lib/types/ReportsState';
import type { StatusTone } from '$lib/tokens';

type StatusLine = {
	/** Short verdict for a badge. */
	label: string;
	tone: StatusTone;
	/** When it last happened ("5m ago"), or `null` when it never did. */
	when: string | null;
};

type PipelineStatus = {
	mailbox: StatusLine;
	reports: StatusLine;
	/** Worst of the two, as one label for the header (the server's `headline` says which). */
	overall: { label: string; tone: StatusTone };
};

const MAILBOX_LABEL: Record<MailboxState, string> = {
	importing: 'Importing',
	syncing: 'Syncing',
	not_configured: 'Not set up',
	error: 'Error',
	awaiting_first_poll: 'Awaiting first poll',
	active: 'Active',
	overdue: 'Overdue',
	stalled: 'Stalled'
};

const REPORTS_LABEL: Record<ReportsState, string> = {
	no_reports: 'No reports yet',
	up_to_date: 'Up to date',
	late: 'Late',
	overdue: 'Overdue'
};

/** The header's label when it names the mailbox (`headline: 'mailbox'`). */
const MAILBOX_HEADLINE: Record<MailboxState, string> = {
	importing: 'Importing',
	syncing: 'Syncing',
	not_configured: 'Not set up',
	error: 'Mailbox error',
	awaiting_first_poll: 'Awaiting first poll',
	active: 'Up to date',
	overdue: 'Mailbox poll overdue',
	stalled: 'Mailbox stalled'
};

/** The header's label when it names the reports (`headline: 'reports'`). */
const REPORTS_HEADLINE: Record<ReportsState, string> = {
	no_reports: 'No reports yet',
	up_to_date: 'Up to date',
	late: 'Reports late',
	overdue: 'Reports overdue'
};

/** The header's one label: the line the server's `headline` names, worded. */
const HEADLINE_LABEL: Record<PipelineHeadline, (p: Pipeline) => string> = {
	mailbox: (p) => MAILBOX_HEADLINE[p.mailbox],
	reports: (p) => REPORTS_HEADLINE[p.reports],
	all_well: () => 'Up to date'
};

/** The server's pipeline verdict (`health.pipeline`), worded. */
export function pipelineStatus(health: SystemHealthResponse): PipelineStatus {
	const p = health.pipeline;
	const mailbox: StatusLine = {
		label: MAILBOX_LABEL[p.mailbox],
		tone: p.mailbox_status,
		when: health.last_check_at != null ? relativeTime(health.last_check_at) : null
	};
	const reports: StatusLine = {
		label: REPORTS_LABEL[p.reports],
		tone: p.reports_status,
		when: health.last_ingest_at != null ? relativeTime(health.last_ingest_at) : null
	};
	// A running import shows its progress; otherwise the line the server names.
	const label = health.imap_progress ? progressLabel(health.imap_progress) : HEADLINE_LABEL[p.headline](p);
	return { mailbox, reports, overall: { label, tone: p.status } };
}

/** One line on the mailbox poller for the setup and the waiting banner. */
export function lastPollText(health: SystemHealthResponse): string {
	if (health.imap_progress) return progressLabel(health.imap_progress);
	const at = health.last_poll.at;
	if (at == null) return 'Mailbox not polled yet';
	return health.last_poll.failed ? `Last poll failed ${relativeTime(at)}` : `Last poll ${relativeTime(at)}`;
}

/**
 * The last mailbox error with its time, for display; `null` while a pass is running (the
 * error is being retried) or when the last pass succeeded.
 */
export function lastErrorText(health: SystemHealthResponse): string | null {
	if (health.imap_progress || !health.imap_configured || !health.imap_last_error) return null;
	if (health.imap_last_error_at == null) return health.imap_last_error;
	return `${formatDateTime(health.imap_last_error_at)}: ${health.imap_last_error}`;
}
