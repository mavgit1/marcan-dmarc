/**
 * App-wide refresh signal. Pages that fetch on `refresh.tick` refetch when a finished import,
 * an action that changed data (a retry, a saved domain change) or the auto-refresh
 * (`AUTO_REFRESH_MS`, a timer the root layout runs through `startAutoRefresh`) bumps it.
 * There is no manual refresh button: data changes only when a mailbox or DNS pass stores
 * something, and those reload what they change.
 */
export const AUTO_REFRESH_MS = 5 * 60 * 1000;
/** How often an overdue auto-refresh looks again while an import pauses it. */
const PAUSED_RECHECK_MS = 30 * 1000;

class RefreshState {
	tick = $state(0);
	lastUpdatedAt = $state<number | null>(null);
	/** When `tick` was last bumped (app start counts), for the auto-refresh countdown. */
	lastRefreshAt = Date.now();

	refresh() {
		this.tick += 1;
		this.lastRefreshAt = Date.now();
	}

	/** Bump `tick` once the last refresh is `AUTO_REFRESH_MS` ago. */
	autoRefresh(now: number = Date.now()) {
		if (now - this.lastRefreshAt >= AUTO_REFRESH_MS) this.refresh();
	}

	/**
	 * The periodic auto-refresh: `AUTO_REFRESH_MS` after the last refresh of any kind,
	 * `autoRefresh()` unless `paused()` (a running import: its progress reloads the dashboard
	 * on its own, and its end refreshes every page). A timer of its own, so it runs whether or
	 * not the health poll answers, re-aimed after every firing at the last refresh. Returns
	 * the stop.
	 */
	startAutoRefresh(paused: () => boolean = () => false): () => void {
		let timer: ReturnType<typeof setTimeout> | undefined;
		const schedule = () => {
			const due = this.lastRefreshAt + AUTO_REFRESH_MS - Date.now();
			// Overdue while paused: look again soon, not in a tight loop.
			const wait = due > 0 ? Math.ceil(due) : paused() ? PAUSED_RECHECK_MS : 0;
			timer = setTimeout(() => {
				if (!paused()) this.autoRefresh();
				schedule();
			}, wait);
		};
		schedule();
		return () => clearTimeout(timer);
	}

	markUpdated(at: number = Date.now()) {
		this.lastUpdatedAt = at;
	}
}

export const refresh = new RefreshState();
