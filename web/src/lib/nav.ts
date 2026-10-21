import type { Component } from 'svelte';
import BellIcon from '@lucide/svelte/icons/bell';
import BookOpenIcon from '@lucide/svelte/icons/book-open';
import Building2Icon from '@lucide/svelte/icons/building-2';
import FileBarChartIcon from '@lucide/svelte/icons/file-bar-chart';
import GlobeIcon from '@lucide/svelte/icons/globe';
import LayoutDashboardIcon from '@lucide/svelte/icons/layout-dashboard';
import MailIcon from '@lucide/svelte/icons/mail';
import NetworkIcon from '@lucide/svelte/icons/network';
import SettingsIcon from '@lucide/svelte/icons/settings';
import { pageFromPath, settingsPage } from './settings-nav';

/** Primary destinations shown in the sidebar. */
type NavHref =
	'/' | '/domains' | '/senders' | '/reports' | '/alerts' | '/organizations' | '/tools/dns' | '/guide' | '/settings';

export type NavItem = {
	href: NavHref;
	title: string;
	icon: Component;
	/** A small `warn` label after the title ("Experimental"), as on the page itself. */
	tag?: string;
};

type NavGroup = {
	label: string;
	items: NavItem[];
};

export const NAV_GROUPS: NavGroup[] = [
	{
		label: 'Monitor',
		items: [
			{ title: 'Dashboard', href: '/', icon: LayoutDashboardIcon },
			{ title: 'Reports', href: '/reports', icon: FileBarChartIcon },
			{ title: 'Alerts', href: '/alerts', icon: BellIcon, tag: 'Experimental' }
		]
	},
	{
		label: 'Inventory',
		items: [
			{ title: 'Domains', href: '/domains', icon: GlobeIcon },
			{ title: 'Senders', href: '/senders', icon: MailIcon },
			{ title: 'Organizations', href: '/organizations', icon: Building2Icon }
		]
	},
	{
		label: 'Tools',
		items: [
			{ title: 'DNS inspector', href: '/tools/dns', icon: NetworkIcon },
			{ title: 'Guide', href: '/guide', icon: BookOpenIcon }
		]
	}
];

/** The public documentation site (setup, deployment, how verdicts are reached). */
export const DOCS_URL = 'https://marcan.kaitiaki.tech/';

/** Pinned to the bottom of the sidebar, apart from the groups. */
export const NAV_FOOTER: NavItem[] = [{ title: 'Settings', href: '/settings', icon: SettingsIcon }];

const NAV_ITEMS: NavItem[] = [...NAV_GROUPS.flatMap((g) => g.items), ...NAV_FOOTER];

export function isNavActive(href: string, pathname: string): boolean {
	if (href === '/') return pathname === '/';
	return pathname === href || pathname.startsWith(`${href}/`);
}

/** Page title for the top bar (nested routes fall back to the section name). */
export function titleForPath(pathname: string): string {
	const exact = NAV_ITEMS.find((n) => n.href === pathname);
	if (exact) return exact.title;

	if (pathname === '/setup') return 'Setup';
	if (pathname.startsWith('/tools/dns')) return 'DNS inspector';
	if (pathname.startsWith('/tools/')) return 'Tools';
	if (pathname.startsWith('/reports/')) return 'Report';
	if (pathname.startsWith('/senders/')) return 'Sender';
	if (pathname.startsWith('/domains/')) return 'Domain';
	if (pathname.startsWith('/organizations/')) return 'Organization';
	if (pathname.startsWith('/settings/')) return 'Settings';
	return 'Marcan';
}

/** One step of the trail; `href` is unresolved (a list, or an organization's page). */
export type Crumb = { label: string; href?: string };

/**
 * The organization the scope is narrowed to (in the header filter, or by a link from the
 * organization's page, which names it: `?org=`), for the trail of a domain or sender page.
 */
export type CrumbOrganization = { id: number; name: string };

/** "Organizations › Acme": the trail's start on a page reached within an organization. */
function organizationCrumbs(organization: CrumbOrganization): Crumb[] {
	return [
		{ label: 'Organizations', href: '/organizations' },
		{ label: organization.name, href: `/organizations/${organization.id}` }
	];
}

/**
 * Breadcrumb trail for the current path. The last item is the current page (no href). With
 * `organization` (the scope is narrowed to one, as by a link from its page), a domain page reads
 * Organizations › organization › domain and a sender page Organizations › organization ›
 * Senders › sender.
 */
export function crumbsForPath(
	pathname: string,
	params?: Record<string, string>,
	entityName?: string | null,
	organization?: CrumbOrganization | null
): Crumb[] {
	if (pathname === '/') return [{ label: 'Dashboard' }];

	if (pathname.startsWith('/tools/dns')) return [{ label: 'Tools' }, { label: 'DNS inspector' }];
	if (pathname.startsWith('/tools')) return [{ label: 'Tools' }];
	if (pathname === '/guide') return [{ label: 'Tools' }, { label: 'Guide' }];
	if (pathname === '/setup') return [{ label: 'Setup' }];

	if (pathname.startsWith('/reports/') && params?.id) {
		return [{ label: 'Reports', href: '/reports' }, { label: entityName || `Report ${params.id}` }];
	}
	if (pathname.startsWith('/senders/') && params?.ip) {
		return [
			...(organization ? organizationCrumbs(organization) : []),
			{ label: 'Senders', href: '/senders' },
			{ label: entityName || decodeURIComponent(params.ip) }
		];
	}
	if (pathname.startsWith('/domains/') && params?.id) {
		return [
			...(organization ? organizationCrumbs(organization) : [{ label: 'Domains', href: '/domains' }]),
			{ label: entityName || `Domain ${params.id}` }
		];
	}
	if (pathname.startsWith('/organizations/') && params?.id) {
		return [{ label: 'Organizations', href: '/organizations' }, { label: entityName || `Organization ${params.id}` }];
	}

	const settings = pageFromPath(pathname);
	if (settings) return [{ label: 'Settings', href: '/settings' }, { label: settingsPage(settings)!.title }];

	const item = NAV_ITEMS.find((n) => n.href === pathname);
	return [{ label: item?.title ?? 'Marcan' }];
}
