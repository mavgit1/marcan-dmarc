/** Entries in a text setting: lines that are neither blank nor `#` comments. */
export function countEntries(text: string): number {
	return text.split('\n').filter((line) => line.trim() && !line.trim().startsWith('#')).length;
}
