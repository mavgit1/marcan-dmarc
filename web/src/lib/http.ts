import ky, { HTTPError, TimeoutError, isNetworkError, isTimeoutError } from 'ky';
import type { ErrorBody } from '$lib/types/ErrorBody';
import type { LineError } from '$lib/types/LineError';

/** Shared ky client. The UI is served same-origin with the API; `pnpm dev` proxies `/api` via Vite. */
export const api = ky.create({
	prefix: '/api',
	timeout: 30_000
});

/** What a query parameter may hold before serialization. */
export type QueryValue = string | number | boolean | null | undefined;

/**
 * The one query serializer: strings trimmed, `null` / `undefined` / empty strings / `false`
 * left out, `true` sent as `true`. Every scope, paging, filter and DNS request goes
 * through it; each endpoint reads the keys it supports. `undefined` when nothing remains.
 */
export function searchParams(params: Record<string, QueryValue> | undefined): Record<string, string> | undefined {
	if (!params) return undefined;
	const out: Record<string, string> = {};
	for (const [key, raw] of Object.entries(params)) {
		const value = typeof raw === 'string' ? raw.trim() : raw;
		if (value === null || value === undefined || value === '' || value === false) continue;
		out[key] = String(value);
	}
	return Object.keys(out).length > 0 ? out : undefined;
}

/** True when ky got a 404 — used for missing report ids. */
export function isNotFound(error: unknown): boolean {
	return error instanceof HTTPError && error.response.status === 404;
}

/** True when ky got a 400 (e.g. a sender path that is not an IP). */
export function isBadRequest(error: unknown): boolean {
	return error instanceof HTTPError && error.response.status === 400;
}

/** True when ky got a 409, e.g. a domain that already exists in some organization. */
export function isConflict(error: unknown): boolean {
	return error instanceof HTTPError && error.response.status === 409;
}

/**
 * The server's error answer (`ErrorBody`): every API error, axum's own rejections included, is
 * JSON with `error` and `code`. `null` for anything else (a network failure, a proxy's page).
 */
export function errorBody(error: unknown): ErrorBody | null {
	if (!(error instanceof HTTPError)) return null;
	// ky 2 consumes the body into `error.data`; `error.response.json()` would throw.
	const body: unknown = error.data;
	if (!body || typeof body !== 'object') return null;
	const { error: message, code } = body as Partial<ErrorBody>;
	return typeof message === 'string' && typeof code === 'string' ? (body as ErrorBody) : null;
}

/** `POST /api/domains` 409: the organization that already has the domain, when the server names it. */
export function conflictOrganizationName(error: unknown): string | null {
	if (!isConflict(error)) return null;
	return errorBody(error)?.organization_name ?? null;
}

/** The per-line `errors` of a refused list (domains, sender names, selectors, recipients), else none. */
export function apiLineErrors(error: unknown): LineError[] {
	return errorBody(error)?.errors ?? [];
}

/** The save bar's line for a rejected text: how many lines to fix, else the error itself. */
export function textSaveError(error: unknown): string {
	const n = apiLineErrors(error).length;
	return n > 0
		? `Fix ${n === 1 ? 'the line' : `the ${n} lines`} marked above; nothing was saved.`
		: apiErrorMessage(error);
}

/**
 * A sentence for the user from anything a request can throw: the server's own `error`
 * message when it sent one, else a plain description (never ky's raw request dump).
 */
export function apiErrorMessage(error: unknown): string {
	if (isTimeoutError(error) || error instanceof TimeoutError) {
		return 'The Marcan server took too long to respond.';
	}
	if (isNetworkError(error)) {
		return 'Could not reach the Marcan server.';
	}
	if (error instanceof HTTPError) {
		const body = errorBody(error);
		if (body) return body.error;
		const { status, statusText } = error.response;
		return `The Marcan server answered ${status}${statusText ? ` ${statusText}` : ''}.`;
	}
	if (error instanceof TypeError && /fetch|network/i.test(error.message)) {
		return 'Could not reach the Marcan server.';
	}
	return error instanceof Error ? error.message : String(error);
}
