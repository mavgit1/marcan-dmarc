/** Splitting what the user typed (lists of domains, selectors, addresses), and joining a list into a sentence. */

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

/** `a`, `a and b`, `a, b and c`. */
export function joinAnd(items: readonly string[]): string {
	if (items.length <= 1) return items[0] ?? '';
	return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

/** A piece of a server sentence: plain words, or literal record text the server marked with backticks. */
export type TextPart = { code: boolean; text: string };

/**
 * A server sentence split at its backtick spans (`v=STSv1; id=…`), so the literal parts can be
 * shown as inline code. An unpaired backtick stays as written, and so does an empty pair.
 */
export function codeSpans(text: string): TextPart[] {
	const parts: TextPart[] = [];
	let last = 0;
	for (const m of text.matchAll(/`([^`]+)`/g)) {
		if (m.index > last) parts.push({ code: false, text: text.slice(last, m.index) });
		parts.push({ code: true, text: m[1] });
		last = m.index + m[0].length;
	}
	if (last < text.length) parts.push({ code: false, text: text.slice(last) });
	return parts;
}
