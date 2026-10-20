// The auto-refresh: a timer of its own that bumps `tick` once the last refresh is
// AUTO_REFRESH_MS ago, never while paused (an import is running), and stops when told to.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AUTO_REFRESH_MS, refresh } from './refresh.svelte';

describe('startAutoRefresh', () => {
	beforeEach(() => {
		vi.useFakeTimers();
		refresh.lastRefreshAt = Date.now();
	});
	afterEach(() => vi.useRealTimers());

	it('refreshes every interval once nothing else did', () => {
		const before = refresh.tick;
		const stop = refresh.startAutoRefresh();
		vi.advanceTimersByTime(AUTO_REFRESH_MS - 1);
		expect(refresh.tick).toBe(before);
		vi.advanceTimersByTime(1);
		expect(refresh.tick).toBe(before + 1);
		vi.advanceTimersByTime(AUTO_REFRESH_MS);
		expect(refresh.tick).toBe(before + 2);
		stop();
	});

	it('counts from the last refresh of any kind', () => {
		const before = refresh.tick;
		const stop = refresh.startAutoRefresh();
		vi.advanceTimersByTime(AUTO_REFRESH_MS / 2);
		refresh.refresh();
		expect(refresh.tick).toBe(before + 1);
		// Exactly AUTO_REFRESH_MS after the manual refresh, not a full period later.
		vi.advanceTimersByTime(AUTO_REFRESH_MS - 1);
		expect(refresh.tick).toBe(before + 1);
		vi.advanceTimersByTime(1);
		expect(refresh.tick).toBe(before + 2);
		stop();
	});

	it('waits while paused and stops when told to', () => {
		const before = refresh.tick;
		let paused = true;
		const stop = refresh.startAutoRefresh(() => paused);
		vi.advanceTimersByTime(AUTO_REFRESH_MS * 2);
		expect(refresh.tick).toBe(before);
		paused = false;
		vi.advanceTimersByTime(AUTO_REFRESH_MS);
		expect(refresh.tick).toBe(before + 1);
		stop();
		vi.advanceTimersByTime(AUTO_REFRESH_MS * 3);
		expect(refresh.tick).toBe(before + 1);
	});
});
