/** Format a Unix timestamp (seconds) as a short local date. */
export function formatDate(ts: number): string {
	return new Date(ts * 1000).toLocaleDateString(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric'
	});
}

/**
 * A calendar day in UTC with the month in words ("1 November 2026" or "November 1, 2026", by
 * locale): the day a client report goes out, which the server sets in UTC.
 */
export function formatDay(ts: number): string {
	return new Date(ts * 1000).toLocaleDateString(undefined, {
		year: 'numeric',
		month: 'long',
		day: 'numeric',
		timeZone: 'UTC'
	});
}

/** Format a Unix timestamp (seconds) as a short local date and time ("Oct 3, 2:15 PM"). */
export function formatDateTime(ts: number): string {
	return new Date(ts * 1000).toLocaleString(undefined, {
		month: 'short',
		day: 'numeric',
		hour: 'numeric',
		minute: '2-digit'
	});
}

/** Format a date range from Unix begin/end seconds. */
export function formatDateRange(begin: number, end: number): string {
	return `${formatDate(begin)} – ${formatDate(end)}`;
}

/** Format an integer with locale grouping. */
export function formatInt(n: number): string {
	return n.toLocaleString();
}

/** Format a byte count in megabytes with one decimal ("12.3 MB"). */
export function formatMegabytes(bytes: number): string {
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Format a percentage with one decimal place. */
export function formatPct(pct: number): string {
	return `${pct.toFixed(1)}%`;
}

/**
 * "1 report" / "3 reports", the count through `formatInt`; `many` for an irregular plural.
 * The one place a count gets its noun: no call site writes `n === 1 ? '' : 's'`.
 */
export function plural(n: number, word: string, many: string = `${word}s`): string {
	return `${formatInt(n)} ${n === 1 ? word : many}`;
}

/** Local time of day ("2:15 PM") of a millisecond timestamp (the header's "Updated"). */
export function formatTimeOfDay(ms: number): string {
	return new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
}

/**
 * "just now" / "5m ago" / "3h ago" / "2d ago" for a unix-seconds timestamp. The one
 * relative-time format of the app (header, settings, setup, DNS grades).
 */
export function relativeTime(ts: number, nowMs: number = Date.now()): string {
	const secs = Math.max(0, Math.floor(nowMs / 1000 - ts));
	if (secs < 60) return 'just now';
	if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
	if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
	return `${Math.floor(secs / 86400)}d ago`;
}

/** Human age of a DNS grade measurement for display next to the letter. */
export function formatCheckedAge(checkedAt: string | number | null | undefined, nowMs: number = Date.now()): string {
	if (checkedAt == null || checkedAt === '') return 'not graded yet';
	const thenMs = typeof checkedAt === 'number' ? checkedAt * 1000 : Date.parse(String(checkedAt));
	if (!Number.isFinite(thenMs)) return 'checked time unknown';
	return `checked ${relativeTime(thenMs / 1000, nowMs)}`;
}

/** The noun alone, "message" / "messages", for a count already shown (e.g. in bold) next to it. */
export function pluralNoun(n: number, word: string, many: string = `${word}s`): string {
	return n === 1 ? word : many;
}

/** `part` as a percentage of `whole` (0 when `whole` is 0); format it with `formatPct`. */
export function percent(part: number, whole: number): number {
	return whole > 0 ? (part * 100) / whole : 0;
}
