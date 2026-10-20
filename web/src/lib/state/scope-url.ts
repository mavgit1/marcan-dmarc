/**
 * The scope filter (organization, domain, time range) in the URL, so a reload or a bookmark
 * keeps it: `org=<id>`, `domain=<id>`, `since=<7d|30d|90d>`, each left out at its default (no
 * organization, no domain, 30 days). Pure: `filters.svelte.ts` applies it on navigation.
 */

/** Time ranges offered by the scope filter; `since` holds one of the values. */
export const SINCE_OPTIONS = [
	{ value: '7d', label: 'Last 7 days', phrase: 'in the last 7 days' },
	{ value: '30d', label: 'Last 30 days', phrase: 'in the last 30 days' },
	{ value: '90d', label: 'Last 90 days', phrase: 'in the last 90 days' }
] as const;

export function sinceLabel(since: string | null | undefined): string {
	return SINCE_OPTIONS.find((o) => o.value === since)?.label ?? (since || 'All time');
}

/** The time range inside a sentence: "failed DMARC in the last 30 days". */
export function sincePhrase(since: string | null | undefined): string {
	return SINCE_OPTIONS.find((o) => o.value === since)?.phrase ?? 'in all reports';
}

export interface Scope {
	organizationId: number | null;
	domainId: number | null;
	since: string;
}

export const DEFAULT_SCOPE: Readonly<Scope> = { organizationId: null, domainId: null, since: '30d' };

/** A scope query parameter. */
export type ScopeParam = 'org' | 'domain' | 'since';

const ALL: readonly ScopeParam[] = ['org', 'domain', 'since'];
const ORG_AND_SINCE: readonly ScopeParam[] = ['org', 'since'];
const SINCE_ONLY: readonly ScopeParam[] = ['since'];
const NONE: readonly ScopeParam[] = [];

/**
 * The parameters a page reads, so its URL carries only those. A page whose route names part of
 * the scope (a domain page its domain, an organization page its organization) leaves it out, and
 * so does a page the scope does not narrow (Settings, the tools, a report).
 */
export function scopeParamsFor(pathname: string): readonly ScopeParam[] {
	const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname;
	if (path === '/' || path === '/alerts' || path === '/reports' || path === '/senders') return ALL;
	// A domain or sender page pins its own entity; it still carries the organization so a
	// reload there, then a step back to a list, keeps the organization the list was narrowed to.
	if (path === '/domains' || path.startsWith('/domains/') || path.startsWith('/senders/')) {
		return ORG_AND_SINCE;
	}
	if (path === '/organizations' || path.startsWith('/organizations/')) {
		return SINCE_ONLY;
	}
	return NONE;
}

function parseId(raw: string | null): number | null {
	if (raw === null || !/^[1-9]\d{0,15}$/.test(raw)) return null;
	const id = Number(raw);
	return Number.isSafeInteger(id) ? id : null;
}

function parseSince(raw: string | null): string | null {
	return SINCE_OPTIONS.some((o) => o.value === raw) ? raw : null;
}

/** The valid values among `params` present in the URL; an invalid one counts as absent. */
export function readScope(search: URLSearchParams, params: readonly ScopeParam[]): Partial<Scope> {
	const found: Partial<Scope> = {};
	if (params.includes('org')) {
		const id = parseId(search.get('org'));
		if (id !== null) found.organizationId = id;
	}
	if (params.includes('domain')) {
		const id = parseId(search.get('domain'));
		if (id !== null) found.domainId = id;
	}
	if (params.includes('since')) {
		const since = parseSince(search.get('since'));
		if (since !== null) found.since = since;
	}
	return found;
}

/**
 * The scope after a navigation to `search`. `reset` (the first load, back and forward): the URL
 * is the source, so a page's parameter that is absent is its default. `carry` (an in-app link
 * or `goto`): a link does not name the scope, so an absent parameter keeps the current value.
 * A parameter the page does not read keeps the current value either way.
 */
export function adoptScope(
	current: Scope,
	search: URLSearchParams,
	params: readonly ScopeParam[],
	mode: 'reset' | 'carry'
): Scope {
	const found = readScope(search, params);
	const base: Scope = { ...current };
	if (mode === 'reset') {
		if (params.includes('org')) base.organizationId = DEFAULT_SCOPE.organizationId;
		if (params.includes('domain')) base.domainId = DEFAULT_SCOPE.domainId;
		if (params.includes('since')) base.since = DEFAULT_SCOPE.since;
	}
	return { ...base, ...found };
}

/**
 * `path` with the parts of the scope it names in its query (`org`, `domain`), for a link that
 * narrows the page it opens: an organization page's links to its domains carry `?org=`, a
 * domain's "all changes" link `?org=…&domain=…`. Opening it narrows the scope (`adoptScope`),
 * visibly in the URL; an in-app link without them carries the current scope.
 */
export function scopedHref(path: string, scope: Partial<Pick<Scope, 'organizationId' | 'domainId'>>): string {
	const params: ScopeParam[] = [];
	if (scope.organizationId != null) params.push('org');
	if (scope.domainId != null) params.push('domain');
	const query = writeScope(new URLSearchParams(), { ...DEFAULT_SCOPE, ...scope }, params).toString();
	return query ? `${path}?${query}` : path;
}

/**
 * `search` with `params` set from `scope`, each left out at its default. Other parameters (a
 * tab, a list's state) and scope parameters the page does not read stay as they are.
 */
export function writeScope(search: URLSearchParams, scope: Scope, params: readonly ScopeParam[]): URLSearchParams {
	const next = new URLSearchParams(search);
	const values: Record<ScopeParam, string | null> = {
		org: scope.organizationId === DEFAULT_SCOPE.organizationId ? null : String(scope.organizationId),
		domain: scope.domainId === DEFAULT_SCOPE.domainId ? null : String(scope.domainId),
		since: scope.since === DEFAULT_SCOPE.since ? null : scope.since
	};
	for (const param of params) {
		const value = values[param];
		if (value === null) next.delete(param);
		else next.set(param, value);
	}
	return next;
}
