import { getSystemHealth, syncImap } from '$lib/api';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import { refresh } from './refresh.svelte';

/** Health poll while no mailbox pass is running. */
const IDLE_POLL_MS = 30_000;
/** Health poll while a mailbox pass runs or new domains are being checked, so progress moves. */
const RUNNING_POLL_MS = 3_000;
/** At most this often, a long import makes the dashboard reload its figures. */
const IMPORT_REFRESH_MIN_MS = 15_000;

/**
 * App-wide `GET /api/system/health`, polled every 30 s, or every 3 s while the mailbox
 * is being imported or domains without a DNS result are being checked. The app header keeps
 * it running; other components read `current` for live progress. A running import bumps
 * `importTick` and its end refreshes every page; new DNS results bump `dnsTick`. The
 * 5-minute auto-refresh is the layout's own timer (`refresh.startAutoRefresh`).
 */
class HealthState {
	current = $state.raw<SystemHealthResponse | null>(null);
	/**
	 * Bumped (throttled) while a running import adds reports. The dashboard reloads on it,
	 * so figures grow during a long import without reloading every other page.
	 */
	importTick = $state(0);
	/**
	 * Bumped (throttled like `importTick`) while domains get their first DNS result, and once
	 * when that ends. Lists showing DNS grades and policies reload on it.
	 */
	dnsTick = $state(0);

	#timer: ReturnType<typeof setTimeout> | null = null;
	#users = 0;
	#request = 0;
	#seenIngested = 0;
	#lastImportRefresh = 0;
	#lastDnsRefresh = 0;

	/** Keep polling while at least one caller is subscribed; returns the unsubscribe. */
	track(): () => void {
		this.#users += 1;
		if (this.#users === 1 && this.#timer == null && this.current) this.#schedule();
		return () => {
			this.#users -= 1;
			if (this.#users === 0) this.#clear();
		};
	}

	/** Fetch now (e.g. right after starting a sync); failures keep the last value. */
	async poll(): Promise<SystemHealthResponse | null> {
		this.#clear();
		const id = ++this.#request;
		try {
			const next = await getSystemHealth();
			if (id === this.#request) this.#apply(next);
		} catch {
			/* keep the last known status */
		} finally {
			if (id === this.#request && this.#users > 0) this.#schedule();
		}
		return this.current;
	}

	/**
	 * "Sync now": start a background pass and poll at once so its progress shows. The
	 * outcome arrives through health polling; only a sync that did not start returns
	 * feedback (already running is not an error).
	 */
	async startSync(): Promise<{ ok: boolean; message: string } | void> {
		const res = await syncImap();
		await this.poll();
		if (res.started) return;
		return { ok: res.reason === 'already_running', message: res.message };
	}

	#apply(next: SystemHealthResponse) {
		const dnsBefore = this.current?.dns_checks ?? null;
		const dnsNow = next.dns_checks;
		if (dnsBefore && dnsNow.checked > dnsBefore.checked) {
			const ended = !dnsNow.checking;
			if (ended || Date.now() - this.#lastDnsRefresh >= IMPORT_REFRESH_MIN_MS) {
				this.#lastDnsRefresh = Date.now();
				this.dnsTick += 1;
			}
		} else if (dnsBefore?.checking && !dnsNow.checking) {
			this.dnsTick += 1;
		}
		const before = this.current?.imap_progress ?? null;
		const now = next.imap_progress;
		this.current = next;
		if (now) {
			if (!before || before.started_at !== now.started_at) this.#seenIngested = 0;
			const due = Date.now() - this.#lastImportRefresh >= IMPORT_REFRESH_MIN_MS;
			if (now.reports_ingested > this.#seenIngested && due) {
				this.#seenIngested = now.reports_ingested;
				this.#lastImportRefresh = Date.now();
				this.importTick += 1;
			}
		} else if (before) {
			// The pass just ended: every page picks up its outcome and final data.
			refresh.refresh();
		}
	}

	#schedule() {
		this.#clear();
		const busy = this.current?.imap_progress || this.current?.dns_checks.checking;
		const ms = busy ? RUNNING_POLL_MS : IDLE_POLL_MS;
		this.#timer = setTimeout(() => void this.poll(), ms);
	}

	#clear() {
		if (this.#timer != null) clearTimeout(this.#timer);
		this.#timer = null;
	}
}

export const systemHealth = new HealthState();
