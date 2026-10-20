// One page per API group: every settings group (`PUT /api/settings/{group}`) is written by exactly
// one Settings page (a page may write several, through its one form). Each model declares the page
// it belongs to and the groups it writes (`writes`); the setup is a view over the same models.
import { describe, expect, it, vi } from 'vitest';
import { SETTINGS_PAGES } from '$lib/settings-nav';
import { AlertsModel } from './alerts-draft.svelte';
import { BrandingModel } from './branding-draft.svelte';
import { ClientReportsModel } from './client-reports-draft.svelte';
import { ImapModel } from './imap-draft.svelte';
import { LicenseModel } from './license-draft.svelte';
import { ResolversModel } from './resolvers-draft.svelte';
import { RetentionModel } from './retention-draft.svelte';
import { SendingModel } from './sending-draft.svelte';
import type { Writes } from './settings-form.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

const MODELS = [
	ImapModel,
	SendingModel,
	AlertsModel,
	ClientReportsModel,
	LicenseModel,
	BrandingModel,
	ResolversModel,
	RetentionModel
];

/** The one writer that is no model: the Data page's "Run setup again" acts at once. */
const ACTIONS: Writes[] = [{ page: 'data', groups: ['onboarding'] }];

/** Which page writes each group; the one place an ownership change is made. */
const OWNER: Record<string, string> = {
	mailbox: 'mailbox',
	reporting: 'mailbox',
	monitoring: 'mailbox',
	retention: 'data',
	dns: 'dns-checks',
	sending: 'sending',
	alerts: 'alerts',
	'client-reports': 'client-reports',
	license: 'license',
	branding: 'branding',
	onboarding: 'data'
};

describe('settings ownership', () => {
	const writers = [...MODELS.map((M) => M.writes), ...ACTIONS];

	it('names pages that exist', () => {
		const pages = SETTINGS_PAGES.map((p) => p.id as string);
		for (const { page } of writers) expect(pages).toContain(page);
	});

	it('has each group written by exactly one Settings page', () => {
		const pages = new Map<string, Set<string>>();
		for (const { page, groups } of writers)
			for (const group of groups) pages.set(group, (pages.get(group) ?? new Set()).add(page));
		const found = Object.fromEntries([...pages].map(([group, owners]) => [group, [...owners].sort().join(' + ')]));
		expect(found).toEqual(OWNER);
	});
});
