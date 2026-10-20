// The hour of a schedule (the digest, the client reports) as the value of an
// `<input type="time">`: settings store a whole hour in UTC, the input shows "07:00".

/** `7` → `07:00`, the input's value for a stored hour. */
export function hourToTime(hour: number): string {
	return `${String(hour).padStart(2, '0')}:00`;
}

/**
 * The hour of an input's value (`07:30` → 7: the minutes are dropped, a schedule runs on the
 * hour); `null` when the field was cleared or holds no time.
 */
export function timeToHour(value: string): number | null {
	const match = /^(\d{1,2}):\d{2}/.exec(value.trim());
	if (!match) return null;
	const hour = Number(match[1]);
	return hour >= 0 && hour <= 23 ? hour : null;
}
