// The scope filter in the URL: which pages carry which parameters, parsing, writing, defaults.
import { describe, expect, it } from 'vitest';
import {
	adoptScope,
	DEFAULT_SCOPE,
	isEntityPath,
	readScope,
	scopedHref,
	scopeParamsFor,
	sincePhrase,
	writeScope,
	type Scope
} from './scope-url';

const ALL = ['org', 'domain', 'since'] as const;
const q = (s: string) => new URLSearchParams(s);
const scope = (patch: Partial<Scope> = {}): Scope => ({ ...DEFAULT_SCOPE, ...patch });

describe('scopeParamsFor', () => {
	it('carries the whole scope on the scoped lists and the dashboard', () => {
		for (const path of ['/', '/alerts', '/reports', '/senders', '/senders/']) {
			expect(scopeParamsFor(path)).toEqual(ALL);
		}
	});

	it('leaves out what a list does not read', () => {
		expect(scopeParamsFor('/domains')).toEqual(['org', 'since']);
		expect(scopeParamsFor('/organizations')).toEqual(['since']);
	});

	it('carries only the time range on a page about one organization, domain or sender', () => {
		for (const path of ['/organizations/3', '/domains/12', '/domains/12/', '/senders/203.0.113.9']) {
			expect(isEntityPath(path)).toBe(true);
			expect(scopeParamsFor(path)).toEqual(['since']);
		}
		for (const path of ['/', '/domains', '/senders', '/organizations', '/reports/5']) {
			expect(isEntityPath(path)).toBe(false);
		}
	});

	it('carries nothing where the scope does not apply', () => {
		for (const path of ['/settings', '/tools/dns', '/guide', '/reports/5', '/nowhere']) {
			expect(scopeParamsFor(path)).toEqual([]);
		}
	});
});

describe('readScope', () => {
	it('parses every parameter', () => {
		expect(readScope(q('org=3&domain=12&since=7d'), ALL)).toEqual({ organizationId: 3, domainId: 12, since: '7d' });
	});

	it('ignores invalid values', () => {
		expect(readScope(q('org=abc&domain=0&since=1y'), ALL)).toEqual({});
		expect(readScope(q('org=-1&domain=1.5&since='), ALL)).toEqual({});
		expect(readScope(q('org=007&domain=99999999999999999999'), ALL)).toEqual({});
	});

	it('reads only the parameters the page carries', () => {
		expect(readScope(q('org=3&domain=12&since=90d'), ['since'])).toEqual({ since: '90d' });
	});
});

describe('adoptScope', () => {
	const current = scope({ organizationId: 3, domainId: 12, since: '7d' });

	it('takes the URL as the source on reset; an absent parameter is its default', () => {
		expect(adoptScope(current, q(''), ALL, 'reset')).toEqual(DEFAULT_SCOPE);
		expect(adoptScope(current, q('since=90d'), ALL, 'reset')).toEqual(scope({ since: '90d' }));
		expect(adoptScope(current, q('org=x'), ALL, 'reset')).toEqual(DEFAULT_SCOPE);
	});

	it('keeps the current scope for an in-app link that does not name it', () => {
		expect(adoptScope(current, q(''), ALL, 'carry')).toEqual(current);
		expect(adoptScope(current, q('since=30d'), ALL, 'carry')).toEqual({ ...current, since: '30d' });
		expect(adoptScope(current, q('since=never'), ALL, 'carry')).toEqual(current);
	});

	it('keeps what the page does not carry, either way', () => {
		expect(adoptScope(current, q('org=5'), ['since'], 'reset')).toEqual({ ...current, since: '30d' });
	});

	it('never changes the scope for opening an organization, domain or sender page', () => {
		const opened = (href: string, current: Scope, mode: 'reset' | 'carry') => {
			const url = new URL(href, 'http://marcan.test');
			return adoptScope(current, url.searchParams, scopeParamsFor(url.pathname), mode);
		};
		for (const current of [scope(), scope({ organizationId: 3, domainId: 12 })]) {
			for (const mode of ['reset', 'carry'] as const) {
				for (const path of ['/organizations/4', '/domains/9', '/senders/192.0.2.1']) {
					expect(opened(path, current, 'carry')).toEqual(current);
					// A stale scope in an old link or bookmark is not adopted.
					expect(opened(`${path}?org=4&domain=9`, current, mode)).toEqual(current);
				}
			}
		}
	});

	it('narrows to what a link to a list names, as picking it in the header does', () => {
		const url = new URL(scopedHref('/alerts', { organizationId: 4, domainId: 9 }), 'http://marcan.test');
		expect(adoptScope(scope(), url.searchParams, scopeParamsFor(url.pathname), 'carry')).toEqual(
			scope({ organizationId: 4, domainId: 9 })
		);
	});
});

describe('scopedHref', () => {
	it('names the parts of the scope it is given', () => {
		expect(scopedHref('/domains', { organizationId: 4 })).toBe('/domains?org=4');
		expect(scopedHref('/alerts', { organizationId: 4, domainId: 9 })).toBe('/alerts?org=4&domain=9');
	});

	it('is the bare path without them', () => {
		expect(scopedHref('/alerts', { organizationId: null })).toBe('/alerts');
		expect(scopedHref('/senders', {})).toBe('/senders');
	});

	it('takes lists only: a link to an organization, domain or sender carries no scope', () => {
		// @ts-expect-error an entity page is not a scoped list
		expect(scopedHref('/domains/9', { organizationId: 4 })).toBe('/domains/9?org=4');
		// @ts-expect-error an entity page is not a scoped list
		expect(scopedHref('/organizations/4', { organizationId: 4 })).toBe('/organizations/4?org=4');
	});
});

describe('writeScope', () => {
	it('omits every default', () => {
		expect(writeScope(q(''), DEFAULT_SCOPE, ALL).toString()).toBe('');
		expect(writeScope(q('org=3&since=7d'), DEFAULT_SCOPE, ALL).toString()).toBe('');
	});

	it('writes what differs from the default', () => {
		expect(writeScope(q(''), scope({ organizationId: 3, since: '7d' }), ALL).toString()).toBe('org=3&since=7d');
		expect(writeScope(q(''), scope({ domainId: 12, since: '90d' }), ALL).toString()).toBe('domain=12&since=90d');
	});

	it('writes only what the page carries and keeps other parameters', () => {
		const s = scope({ organizationId: 3, domainId: 12, since: '7d' });
		expect(writeScope(q('tab=senders'), s, ['since']).toString()).toBe('tab=senders&since=7d');
	});

	it('drops a scope parameter the page does not read, so a domain URL keeps no stale org', () => {
		const s = scope({ organizationId: 3, domainId: 12 });
		expect(writeScope(q('org=18&tab=senders'), s, scopeParamsFor('/domains/9')).toString()).toBe('tab=senders');
		expect(writeScope(q('org=18&domain=9&since=7d'), s, scopeParamsFor('/domains')).toString()).toBe('org=3');
	});

	it('round-trips through readScope', () => {
		const s = scope({ organizationId: 3, domainId: 12, since: '90d' });
		expect(adoptScope(DEFAULT_SCOPE, writeScope(q(''), s, ALL), ALL, 'reset')).toEqual(s);
	});
});

describe('sincePhrase', () => {
	it('reads the range inside a sentence', () => {
		expect(sincePhrase('30d')).toBe('in the last 30 days');
		expect(sincePhrase('7d')).toBe('in the last 7 days');
		expect(sincePhrase(null)).toBe('in all reports');
	});
});
