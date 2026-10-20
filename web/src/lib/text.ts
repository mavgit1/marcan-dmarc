/** Splitting what the user typed: lists of domains, selectors, addresses. */

/** The entries of a list typed free-form: one per line, or separated by commas, semicolons or spaces. */
export function listTokens(raw: string | null | undefined): string[] {
	return (raw ?? '')
		.split(/[\s,;]+/)
		.map((s) => s.trim())
		.filter(Boolean);
}

/** A domain name as the API expects it: lower case, no trailing dot. */
export function normalizeDomain(raw: string): string {
	return raw.trim().toLowerCase().replace(/\.$/, '');
}

/** Without duplicates, in first-seen order. */
export function unique<T>(items: readonly T[]): T[] {
	return [...new Set(items)];
}

/** The text with its first letter upper case, for a phrase that opens a sentence or a title. */
export function capitalize(text: string): string {
	return text.charAt(0).toUpperCase() + text.slice(1);
}
