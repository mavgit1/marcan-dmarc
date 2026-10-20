// The hour of a schedule as a time input's value and back: minutes are dropped, a cleared
// field is no hour.
import { describe, expect, it } from 'vitest';
import { hourToTime, timeToHour } from './time-of-day';

describe('hourToTime / timeToHour', () => {
	it('shows a stored hour on the hour', () => {
		expect(hourToTime(0)).toBe('00:00');
		expect(hourToTime(7)).toBe('07:00');
		expect(hourToTime(23)).toBe('23:00');
	});

	it('reads the hour and drops the minutes', () => {
		expect(timeToHour('07:00')).toBe(7);
		expect(timeToHour('07:45')).toBe(7);
		expect(timeToHour('23:59:30')).toBe(23);
		expect(timeToHour('00:00')).toBe(0);
	});

	it('reads nothing from a cleared or odd value', () => {
		expect(timeToHour('')).toBeNull();
		expect(timeToHour('7')).toBeNull();
		expect(timeToHour('24:00')).toBeNull();
		expect(timeToHour('noon')).toBeNull();
	});

	it('round-trips every hour', () => {
		for (let h = 0; h < 24; h++) expect(timeToHour(hourToTime(h))).toBe(h);
	});
});
