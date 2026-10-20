/**
 * Helpers for pointing DMARC aggregate reports (`rua=`) at the Marcan mailbox: the words for the
 * server's reporting setup (`GET /api/reporting-setup`), which decides each domain's status from
 * live lookups and writes the records to publish (`record_to_publish`; the Enforcement tab's
 * steps carry theirs too). No DMARC record text is parsed or edited here.
 */

import type { DomainAssessment } from '$lib/types/DomainAssessment';
import type { MailboxAuthorization } from '$lib/types/MailboxAuthorization';
import type { ReportingMode } from '$lib/types/ReportingMode';
import type { ReportingStatus } from '$lib/types/ReportingStatus';

/** How long receivers usually take to send the first aggregate report. */
export const FIRST_REPORT_ETA = '24–48 h';

/** Reports arrive: the server's `assessment.reporting`. */
export function isReporting(d: { assessment: Pick<DomainAssessment, 'reporting'> }): boolean {
	return d.assessment.reporting.state === 'reporting';
}

/** Loose `local@domain.tld` shape check; the server validates for real. */
export function looksLikeEmail(value: string): boolean {
	return /^[^\s@;,!<>]+@[^\s@;,!<>]+\.[^\s@;,!<>.]+$/.test(value);
}

/** Lowercased domain part of an email address, or `null`. */
export function emailDomain(address: string): string | null {
	const at = address.lastIndexOf('@');
	if (at < 1) return null;
	return (
		address
			.slice(at + 1)
			.trim()
			.replace(/\.$/, '')
			.toLowerCase() || null
	);
}

/** A domain's reporting status, as the server decided it (`ReportingDomain.state`). */
export const REPORTING_STATUS_LABEL: Record<ReportingStatus, string> = {
	no_record: 'No record',
	not_authorized: 'Authorization missing',
	address_missing: 'Report address missing',
	check_failed: 'Check failed',
	authorization_check_failed: 'Check failed',
	waiting_for_first_report: 'Waiting for first report',
	waiting_for_reports: 'Waiting for reports',
	reporting: 'Reporting'
};

/** The mailbox domain's authorization of other domains' reports; `null`: no badge. */
export const MAILBOX_AUTHORIZATION_LABEL: Record<MailboxAuthorization, string | null> = {
	not_needed: null,
	wildcard: 'Authorized',
	per_domain: 'Authorized per domain',
	no_wildcard: 'No wildcard',
	check_failed: 'Check failed'
};

export type { ReportingMode };
