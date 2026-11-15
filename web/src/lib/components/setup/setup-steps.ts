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
	/** The server's reporting setup; `null` until it answered. */
	reporting: ReportingSetupResponse | null;
	/** The organization step was passed in this visit (keeping "Default" is a valid choice). */
	organizationConfirmed: boolean;
};

export type StepId = 'organization' | 'accounts' | 'import' | 'domains' | 'addresses' | 'dmarc' | 'done';

export type SetupStep = {
	id: StepId;
	/** Its name in the step list. */
	title: string;
	/** Its heading on screen. */
	heading: string;
	done: (d: SetupData) => boolean;
};

/** An account is read: it receives, with its server and login set and readable (the accounts step is done). */
export const accountReady = (s: SettingsResponse) => s.accounts.some((a) => a.ready_to_receive);

/** A mailbox pass is running. */
export const importing = (d: SetupData) => d.health.pass_progress != null;

/** Every address domain accepts the domains' reports, or needs to accept none (the server's verdict). */
const addressesAuthorized = (r: ReportingSetupResponse) =>
	r.authorizations.every((a) => a.state === 'not_needed' || a.status === 'pass');

export const SETUP_STEPS: readonly SetupStep[] = [
	{
		id: 'organization',
		title: 'Organization',
		heading: 'Default organization',
		// Passed in this visit, or behind a connected account (the step after it).
		done: (d) => d.organizationConfirmed || accountReady(d.settings)
	},
	{
		id: 'accounts',
		title: 'Accounts',
		heading: 'The accounts reports arrive in',
		done: (d) => accountReady(d.settings)
	},
	{
		id: 'import',
		title: 'Import',
		heading: 'Import the reports',
		// Every receiving account's last pass finished without error, and none runs now.
		done: (d) =>
			accountReady(d.settings) &&
			!importing(d) &&
			d.health.accounts.length > 0 &&
			d.health.accounts.every((a) => a.last_success_at != null && a.last_error == null)
	},
	{ id: 'domains', title: 'Domains', heading: 'Your domains', done: (d) => d.domains.length > 0 },
	{
		id: 'addresses',
		title: 'Report addresses',
		heading: 'Report addresses',
		done: (d) => !!d.reporting && d.reporting.addresses.length > 0 && addressesAuthorized(d.reporting)
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

/** The steps' ids before 0.4, which links and bookmarks may still name (`?step=`). */
const RENAMED: Record<string, StepId> = { mailbox: 'accounts', delivery: 'addresses' };

/** The index of the step a link names (`?step=`, an old id mapped to its new one); -1 for none. */
export function linkedStep(param: string | null): number {
	if (!param) return -1;
	const id = RENAMED[param] ?? param;
	return SETUP_STEPS.findIndex((s) => s.id === id);
}

/** The step to open: the first that is not done, else the last. */
export function firstOpenStep(d: SetupData): number {
	const i = SETUP_STEPS.findIndex((s) => !s.done(d));
	return i === -1 ? SETUP_STEPS.length - 1 : i;
}
