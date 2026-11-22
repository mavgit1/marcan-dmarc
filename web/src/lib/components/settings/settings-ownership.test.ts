// One page per API group: every settings group (`PUT /api/settings/{group}`) and every resource of
// its own (`/api/accounts`) is written by exactly one Settings page (a page may write several,
// through its one form). Each model declares the page it belongs to and what it writes
// (`writes`); the setup is a view over the same models.
import { describe, expect, it, vi } from 'vitest';
import { SETTINGS_PAGES } from '#lib/settings-nav.ts';
import { AccountModel } from './account-draft.svelte';
import { AlertsModel } from './alerts-draft.svelte';
import { BrandingModel } from './branding-draft.svelte';
import { ClientReportsModel } from './client-reports-draft.svelte';
import { LicenseModel } from './license-draft.svelte';
import { ReceivingModel } from './receiving-draft.svelte';
import { ResolversModel } from './resolvers-draft.svelte';
import { RetentionModel } from './retention-draft.svelte';
import { SendingModel } from './sending-draft.svelte';
import { UpdatesModel } from './updates-draft.svelte';
import type { Writes } from './settings-form.svelte';

vi.mock('$app/paths', () => ({ resolve: (path: string) => (path.startsWith('/') ? path : `/${path}`) }));

const MODELS = [
	AccountModel,
	ReceivingModel,
	SendingModel,
	AlertsModel,
	ClientReportsModel,
	LicenseModel,
	BrandingModel,
	ResolversModel,
	RetentionModel,
	UpdatesModel
];

/** The one writer that is no model: the Data page's "Run setup again" acts at once. */
const ACTIONS: Writes[] = [{ page: 'data', groups: ['onboarding'] }];

/** Which page writes each group and resource; the one place an ownership change is made. */
const OWNER: Record<string, string> = {
	accounts: 'accounts',
	receiving: 'accounts',
	monitoring: 'accounts',
	retention: 'data',
	dns: 'dns-checks',
	sending: 'sending',
	alerts: 'alerts',
	'client-reports': 'client-reports',
	license: 'license',
	branding: 'branding',
	onboarding: 'data',
	updates: 'license'
};

describe('settings ownership', () => {
	const writers = [...MODELS.map((M) => M.writes), ...ACTIONS];

	it('names pages that exist', () => {
		const pages = SETTINGS_PAGES.map((p) => p.id as string);
		for (const { page } of writers) expect(pages).toContain(page);
	});

	it('has each group and resource written by exactly one Settings page', () => {
		const pages = new Map<string, Set<string>>();
		for (const w of writers)
			for (const group of 'groups' in w ? w.groups : [w.resource])
				pages.set(group, (pages.get(group) ?? new Set()).add(w.page));
		const found = Object.fromEntries([...pages].map(([group, owners]) => [group, [...owners].sort().join(' + ')]));
		expect(found).toEqual(OWNER);
	});
});
