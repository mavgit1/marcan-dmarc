/**
 * The setup's steps, in their one order, each with when it is done: read from what the server
 * stored and answers (settings, health, domains, the reporting setup), never kept apart.
 */

import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import type { ScopeDomain } from '$lib/types/ScopeDomain';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';

/** What every step reads. */
export type SetupData = {
	organization: OrganizationSummary;
	/** The monitored domains, names only (`GET /api/scope`). */
	domains: ScopeDomain[];
	settings: SettingsResponse;
	health: SystemHealthResponse;
	/** The server's reporting setup; `null` until it answered (or without a report address). */
	reporting: ReportingSetupResponse | null;
	/** The organization step was passed in this visit (keeping "Default" is a valid choice). */
	organizationConfirmed: boolean;
};

export type StepId = 'organization' | 'mailbox' | 'import' | 'domains' | 'delivery' | 'dmarc' | 'done';

export type SetupStep = {
	id: StepId;
	/** Its name in the step list. */
	title: string;
	/** Its heading on screen. */
	heading: string;
	done: (d: SetupData) => boolean;
};

const mailboxSaved = (d: SetupData) => d.settings.mailbox.configured && d.settings.reporting.report_address !== '';

/** A mailbox pass is running. */
export const importing = (d: SetupData) => d.health.imap_progress != null;

export const SETUP_STEPS: readonly SetupStep[] = [
	{
		id: 'organization',
		title: 'Organization',
		heading: 'Default organization',
		// Passed in this visit, or behind a connected mailbox (the step before it).
		done: (d) => d.organizationConfirmed || d.settings.mailbox.configured
	},
	{ id: 'mailbox', title: 'Mailbox', heading: 'Connect the report mailbox', done: mailboxSaved },
	{
		id: 'import',
		title: 'Import',
		heading: 'Import the reports',
		// The last pass finished without error, and none runs now.
		done: (d) => mailboxSaved(d) && !importing(d) && d.health.last_poll.at != null && !d.health.last_poll.failed
	},
	{ id: 'domains', title: 'Domains', heading: 'Your domains', done: (d) => d.domains.length > 0 },
	{
		id: 'delivery',
		title: 'Report delivery',
		heading: 'Report delivery',
		done: (d) => d.settings.mailbox.configured && d.settings.reporting.reporting_mode_saved
	},
	{
		id: 'dmarc',
		title: 'DMARC records',
		heading: 'Point DMARC reports at Marcan',
		// Every domain set up, as the server decided.
		done: (d) => !!d.reporting && d.reporting.domains.length > 0 && d.reporting.domains.every((r) => r.set_up)
	},
	{
		id: 'done',
		title: 'Done',
		heading: 'Done',
		// Finished, and nothing before it is still open: a finish does not tick open steps.
		done: (d) => d.settings.onboarding.state === 'completed' && SETUP_STEPS.slice(0, -1).every((s) => s.done(d))
	}
];

/** The step to open: the first that is not done, else the last. */
export function firstOpenStep(d: SetupData): number {
	const i = SETUP_STEPS.findIndex((s) => !s.done(d));
	return i === -1 ? SETUP_STEPS.length - 1 : i;
}
