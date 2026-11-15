/**
 * A DNS answer's `checked_at` (ISO 8601) in the stored check's unit, whole unix seconds, so the
 * domain page can tell whether its overview and its scorecard show the same check; `null`
 * when it does not parse.
 */
export function dnsCheckedAt(iso: string): number | null {
	const ms = Date.parse(iso);
	return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}
