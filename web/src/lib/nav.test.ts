// The top bar's title and breadcrumb trail, derived from the path (and the loaded entity's name).
import { describe, expect, it, vi } from 'vitest';
import { NAV_GROUPS, crumbsForPath, isNavActive, titleForPath } from './nav';

vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

describe('NAV_GROUPS', () => {
	it('orders the sidebar groups and their pages', () => {
		expect(NAV_GROUPS.map((g) => [g.label, g.items.map((i) => i.title)])).toEqual([
			['Monitor', ['Dashboard', 'Reports', 'Alerts']],
			['Inventory', ['Domains', 'Organizations', 'Senders']],
			['Tools', ['DNS inspector', 'Guide']]
		]);
	});
});

describe('isNavActive', () => {
	it('matches the dashboard exactly and sections by prefix', () => {
		expect(isNavActive('/', '/')).toBe(true);
		expect(isNavActive('/', '/domains')).toBe(false);
		expect(isNavActive('/domains', '/domains')).toBe(true);
		expect(isNavActive('/domains', '/domains/12')).toBe(true);
		expect(isNavActive('/domains', '/domainsx')).toBe(false);
	});
});

describe('titleForPath', () => {
	it('names every sidebar destination', () => {
		expect(titleForPath('/')).toBe('Dashboard');
		expect(titleForPath('/reports')).toBe('Reports');
		expect(titleForPath('/alerts')).toBe('Alerts');
		expect(titleForPath('/tools/dns')).toBe('DNS inspector');
		expect(titleForPath('/guide')).toBe('Guide');
		expect(titleForPath('/setup')).toBe('Setup');
		expect(titleForPath('/settings')).toBe('Settings');
	});

	it('falls back to the section name on nested routes', () => {
		expect(titleForPath('/reports/5')).toBe('Report');
		expect(titleForPath('/senders/203.0.113.9')).toBe('Sender');
		expect(titleForPath('/domains/3')).toBe('Domain');
		expect(titleForPath('/organizations/2')).toBe('Organization');
		expect(titleForPath('/settings/alerts')).toBe('Settings');
		expect(titleForPath('/tools/other')).toBe('Tools');
		expect(titleForPath('/nowhere')).toBe('Marcan');
	});
});

describe('crumbsForPath', () => {
	it('is one crumb on a top-level page', () => {
		expect(crumbsForPath('/')).toEqual([{ label: 'Dashboard' }]);
		expect(crumbsForPath('/domains')).toEqual([{ label: 'Domains' }]);
		expect(crumbsForPath('/guide')).toEqual([{ label: 'Tools' }, { label: 'Guide' }]);
		expect(crumbsForPath('/setup')).toEqual([{ label: 'Setup' }]);
		expect(crumbsForPath('/settings')).toEqual([{ label: 'Settings' }]);
	});

	it('puts a Settings page under the Settings list', () => {
		expect(crumbsForPath('/settings/dns-checks')).toEqual([
			{ label: 'Settings', href: '/settings' },
			{ label: 'DNS checks' }
		]);
	});

	it('puts an account under Accounts, named once known', () => {
		const accounts = [
			{ label: 'Settings', href: '/settings' },
			{ label: 'Accounts', href: '/settings/accounts' }
		];
		expect(crumbsForPath('/settings/accounts/new')).toEqual([...accounts, { label: 'New account' }]);
		expect(crumbsForPath('/settings/accounts/4', { id: '4' }, 'Reports')).toEqual([...accounts, { label: 'Reports' }]);
		expect(crumbsForPath('/settings/accounts/4', { id: '4' }, null)).toEqual([...accounts, { label: 'Account 4' }]);
	});

	it('links back to the list and names the entity, else its id', () => {
		expect(crumbsForPath('/senders/2001%3Adb8%3A%3A1', { ip: '2001%3Adb8%3A%3A1' })).toEqual([
			{ label: 'Senders', href: '/senders' },
			{ label: '2001:db8::1' }
		]);
		expect(crumbsForPath('/organizations/2', { id: '2' }, 'Acme')).toEqual([
			{ label: 'Organizations', href: '/organizations' },
			{ label: 'Acme' }
		]);
		expect(crumbsForPath('/reports/7', { id: '7' })).toEqual([
			{ label: 'Reports', href: '/reports' },
			{ label: 'Report 7' }
		]);
	});

	it('always puts a domain under its organization, also with only one', () => {
		const acme = { id: 2, name: 'Acme' };
		expect(crumbsForPath('/domains/3', { id: '3' }, 'example.com', acme)).toEqual([
			{ label: 'Organizations', href: '/organizations' },
			{ label: 'Acme', href: '/organizations/2' },
			{ label: 'example.com' }
		]);
		// Before the domain loads: the hierarchy's start, the id.
		expect(crumbsForPath('/domains/3', { id: '3' }, null, null)).toEqual([
			{ label: 'Organizations', href: '/organizations' },
			{ label: 'Domain 3' }
		]);
	});

	it('shows the hierarchy, never the organization a sender or other page was reached from', () => {
		const acme = { id: 2, name: 'Acme' };
		expect(crumbsForPath('/senders/192.0.2.1', { ip: '192.0.2.1' }, null, acme)).toEqual([
			{ label: 'Senders', href: '/senders' },
			{ label: '192.0.2.1' }
		]);
		expect(crumbsForPath('/organizations/2', { id: '2' }, 'Acme', acme)).toEqual([
			{ label: 'Organizations', href: '/organizations' },
			{ label: 'Acme' }
		]);
		expect(crumbsForPath('/reports/7', { id: '7' }, null, acme)).toEqual([
			{ label: 'Reports', href: '/reports' },
			{ label: 'Report 7' }
		]);
	});
});
