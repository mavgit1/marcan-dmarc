/**
 * The ingestion pipeline's status from `GET /api/system/health`: is the mailbox being
 * polled, and are reports arriving? The header, Settings and the setup all read
 * it from here so they cannot disagree.
 */

import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { formatDateTime, plural, relativeTime } from '$lib/format';
import { progressLabel } from '$lib/pass-progress';
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

/** A mailbox's state as a label (the header's popover, the Accounts cards). */
export const MAILBOX_LABEL: Record<MailboxState, string> = {
	importing: 'Importing',
	syncing: 'Checking',
	not_configured: 'Not set up',
	error: 'Error',
	awaiting_first_poll: 'Awaiting first check',
	active: 'Active',
	overdue: 'Overdue',
	stalled: 'Stalled'
};

const REPORTS_LABEL: Record<ReportsState, string> = {
	no_reports: 'No reports yet',
	importing: 'Importing',
	up_to_date: 'Up to date',
	late: 'Late',
	overdue: 'Overdue'
};

/** The header's label when it names the mailbox (`headline: 'mailbox'`). */
const MAILBOX_HEADLINE: Record<MailboxState, string> = {
	importing: 'Importing',
	syncing: 'Checking',
	not_configured: 'Not set up',
	error: 'Account error',
	awaiting_first_poll: 'Awaiting first check',
	active: 'Up to date',
	overdue: 'Account check overdue',
	stalled: 'Account check stalled'
};

/** The header's label when it names the reports (`headline: 'reports'`). */
const REPORTS_HEADLINE: Record<ReportsState, string> = {
	no_reports: 'No reports yet',
	importing: 'Importing',
	up_to_date: 'Up to date',
	late: 'Reports late',
	overdue: 'Reports overdue'
};

/** The header's one label: the line the server's `headline` names, worded. */
const HEADLINE_LABEL: Record<PipelineHeadline, (p: Pipeline) => string> = {
	mailbox: (p) => MAILBOX_HEADLINE[p.mailbox],
	reports: (p) => REPORTS_HEADLINE[p.reports],
	importing: () => 'Importing',
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
	// The messages left are known while a pass downloads; between the passes of a long
	// import they are not, until the next pass has scanned.
	const progress = health.pass_progress;
	const left =
		p.reports === 'importing' && progress?.phase === 'importing'
			? Math.max(0, progress.total_messages - progress.processed_messages)
			: null;
	const reports: StatusLine = {
		label: left != null ? `Importing, ${plural(left, 'message')} left` : REPORTS_LABEL[p.reports],
		tone: p.reports_status,
		when: health.last_ingest_at != null ? relativeTime(health.last_ingest_at) : null
	};
	// A running import shows its progress; otherwise the line the server names.
	const label = health.pass_progress ? progressLabel(health.pass_progress) : HEADLINE_LABEL[p.headline](p);
	return { mailbox, reports, overall: { label, tone: p.status } };
}

/** One line on the mailbox poller for the setup and the waiting banner. */
export function lastPollText(health: SystemHealthResponse): string {
	if (health.pass_progress) return progressLabel(health.pass_progress);
	const at = health.last_poll.at;
	if (at == null) return 'No account checked yet';
	return health.last_poll.failed ? `Last check failed ${relativeTime(at)}` : `Last check ${relativeTime(at)}`;
}

/**
 * The last mailbox error with its time, for display; `null` while a pass is running (the
 * error is being retried) or when the last pass succeeded.
 */
export function lastErrorText(health: SystemHealthResponse): string | null {
	if (health.pass_progress || !health.receive_configured || !health.receive_last_error) return null;
	if (health.receive_last_error_at == null) return health.receive_last_error;
	return `${formatDateTime(health.receive_last_error_at)}: ${health.receive_last_error}`;
}
