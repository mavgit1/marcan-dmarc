import { connectImap, saveMailbox, saveMonitoring, saveReporting } from '$lib/api';
import { apiErrorMessage } from '$lib/http';
import { looksLikeEmail } from '$lib/report-address';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import { systemHealth } from '$lib/state/system-health.svelte';
import type { ConnectImapResponse } from '$lib/types/ConnectImapResponse';
import type { ImapFolderResponse } from '$lib/types/ImapFolderResponse';
import type { ImapSecurity } from '$lib/types/ImapSecurity';
import type { ReportingMode } from '$lib/types/ReportingMode';
import type { SaveMailboxRequest } from '$lib/types/SaveMailboxRequest';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { defaultPort, portAfterSecurityChange, sameAccount } from './connection';
import { SettingsForm, type SettingsFormSource, type Writes } from './settings-form.svelte';

/** The editable fields of the mailbox. */
export type ImapDraft = {
	host: string;
	port: number;
	security: ImapSecurity;
	username: string;
	/** Typed to change it; empty keeps the stored one. */
	password: string;
	/** Empty when reports go to the login address. */
	reportAddress: string;
	folder: string;
	mode: ReportingMode;
	pollMinutes: number;
	/** The mailbox warning: hours without any report. */
	warnHours: number;
	/** The domain warning: days without a report for one domain. */
	stoppedDays: number;
};

export const WARN_HOURS = [4, 12, 24, 48, 72];

/** The domain warning's choices; a stored value outside them is shown too. */
export const STOPPED_DAYS = [2, 3, 5, 7, 10, 14, 30];

/** Shown next to the password once host, port or username differ from the saved mailbox. */
export const MAILBOX_CHANGED_HINT = 'Different mailbox: enter its password.';

export const BLANK: ImapDraft = {
	host: '',
	port: 0,
	security: 'tls',
	username: '',
	password: '',
	reportAddress: '',
	folder: '',
	mode: 'direct',
	pollMinutes: 0,
	warnHours: 0,
	stoppedDays: 0
};

/**
 * The mailbox, reporting and monitoring groups as one draft (the Mailbox page shows them
 * together). The server applies its defaults (`mailbox.defaults`) to unset fields.
 */
export function fromServer(s: SettingsResponse): ImapDraft {
	const { mailbox, reporting, monitoring } = s;
	const sameAsLogin = reporting.report_address.toLowerCase() === mailbox.username.trim().toLowerCase();
	return {
		host: mailbox.host,
		// The field always shows a port: the usual one for the security when none is stored.
		port: mailbox.port || defaultPort(mailbox.security, mailbox.defaults),
		security: mailbox.security,
		username: mailbox.username,
		password: '',
		reportAddress: sameAsLogin ? '' : reporting.report_address,
		folder: mailbox.folder,
		mode: reporting.reporting_mode ?? 'direct',
		pollMinutes: Math.max(1, Math.round(mailbox.poll_interval_secs / 60)),
		warnHours: monitoring.report_freshness_warning_hours,
		stoppedDays: monitoring.domain_reports_stopped_days
	};
}

/** The draft as it would be saved: trimmed, numbers as numbers, the login address as "". */
export function normalize(d: ImapDraft): ImapDraft {
	const username = d.username.trim();
	const reportAddress = d.reportAddress.trim();
	return {
		...d,
		host: d.host.trim(),
		port: Number(d.port),
		username,
		reportAddress: reportAddress.toLowerCase() === username.toLowerCase() ? '' : reportAddress,
		pollMinutes: Number(d.pollMinutes),
		warnHours: Number(d.warnHours),
		stoppedDays: Number(d.stoppedDays)
	};
}

/** The login a draft would try: what a successful test vouches for (the password only when typed). */
export function connectionKey(d: ImapDraft): string {
	return JSON.stringify([d.host.toLowerCase(), d.port, d.security, d.username, d.password]);
}

/** Where reports go: the typed address, else the login when it is one. */
export function effectiveAddress(d: ImapDraft): string {
	return d.reportAddress || (looksLikeEmail(d.username) ? d.username : '');
}

/**
 * Why a normalized draft cannot be saved, worded for the save bar (the setup disables its button
 * instead). Client-side because a save starts with a login to the server on screen: this holds
 * that request (and the save) until the connection is complete enough to try.
 */
export function problemWith(d: ImapDraft, passwordRequired: boolean): string | null {
	if (!d.host) return 'Enter the server host.';
	if (!(d.port > 0)) return 'Enter the server port.';
	if (!d.username) return 'Enter the username.';
	if (passwordRequired && !d.password.trim()) return 'Enter the password.';
	if (d.reportAddress && !looksLikeEmail(d.reportAddress)) return 'The report address is not a valid email address.';
	if (!effectiveAddress(d)) return 'The login is not an email address: enter the report address.';
	if (!(d.pollMinutes >= 1)) return 'Check the mailbox at least every minute.';
	return null;
}

/** The whole mailbox for a normalized draft, the password only when typed. */
function mailboxRequest(d: ImapDraft, pollIntervalSecs: number): SaveMailboxRequest {
	return {
		host: d.host,
		port: d.port,
		security: d.security,
		username: d.username,
		password: d.password || undefined,
		folder: d.folder,
		poll_interval_secs: pollIntervalSecs
	};
}

export type ImapModelOptions = {
	/** The shared settings (Settings) or handed in by the parent (a setup step). */
	source: SettingsFormSource<SettingsResponse>;
	/** Persist the normalized draft (`saveConnection`, `saveDelivery`, `saveSettings` below, wrapped by the form). */
	save: (draft: ImapDraft, saved: SettingsResponse) => Promise<{ saved: SettingsResponse; message?: string }>;
	onLoad?: (saved: SettingsResponse, first: boolean) => void;
};

/**
 * The mailbox model behind every IMAP form, over the saved mailbox: what counts as a changed
 * mailbox, when a password is needed, the effective report address, the login check (folders
 * and the examined folder) and the three saves. Each save writes its groups through
 * `settings.save`, so every reader of the shared settings sees it. Its `problem` does not hold
 * the Mailbox page's Save: a save says what is missing (the setup holds its button instead).
 * Create it during component initialisation (it owns effects). A new mailbox form is another thin
 * component over this model, never a variant prop on an existing one.
 */
export class ImapModel extends SettingsForm<SettingsResponse, ImapDraft> {
	static readonly writes: Writes = { page: 'mailbox', groups: ['mailbox', 'reporting', 'monitoring'] };
	/** Folders the last successful login listed. */
	folders = $state.raw<ImapFolderResponse[] | null>(null);
	/** The last login check's answer (`ok: false` with the reason when it failed). */
	probe = $state.raw<ConnectImapResponse | null>(null);
	probing = $state(false);
	#seq = 0;
	/** `connectionKey` of the last successful login. */
	#loggedIn = $state<string | null>(null);

	/** Host, port or username differ from the saved mailbox, so its password does not apply. */
	mailboxChanged: boolean = $derived.by(() => !!this.base && !sameAccount(this.normalized, this.base));
	passwordRequired: boolean = $derived.by(() => !this.saved?.mailbox.has_password || this.mailboxChanged);
	loginIsAddress: boolean = $derived.by(() => looksLikeEmail(this.normalized.username));
	effectiveAddress: string = $derived.by(() => effectiveAddress(this.normalized));
	canTest: boolean = $derived.by(
		() => !!this.normalized.host && !!this.normalized.username && !(this.passwordRequired && !this.normalized.password)
	);
	/** The last successful login used the connection on screen, so its folder list applies. */
	loginChecked: boolean = $derived.by(
		() => this.#loggedIn !== null && this.#loggedIn === connectionKey(this.normalized)
	);
	/** The examined folder's state, once the check answered for the folder now selected. */
	folderStatus: ConnectImapResponse['selected'] = $derived.by(() =>
		!this.probing && this.probe?.ok && this.probe.selected?.name === this.normalized.folder ? this.probe.selected : null
	);

	#save: ImapModelOptions['save'];

	constructor(options: ImapModelOptions) {
		super({ ...options.source, label: 'Mailbox', blank: BLANK, secret: 'password', onLoad: options.onLoad });
		this.#save = options.save;
	}

	protected toDraft(s: SettingsResponse): ImapDraft {
		return fromServer(s);
	}

	protected normalize(d: ImapDraft): ImapDraft {
		return normalize(d);
	}

	protected check(d: ImapDraft): string | null {
		return problemWith(d, this.passwordRequired);
	}

	get canSave(): boolean {
		return true;
	}

	protected write(d: ImapDraft, s: SettingsResponse): Promise<{ saved: SettingsResponse; message?: string }> {
		return this.#save(d, s);
	}

	/**
	 * Login check: lists the folders and examines `name`. Typed credentials are sent only when
	 * they differ from the saved mailbox; otherwise the server uses the stored password. `null`
	 * when a later check superseded this one.
	 */
	async checkLogin(name: string): Promise<ConnectImapResponse | null> {
		const id = ++this.#seq;
		this.probing = true;
		try {
			const draft = this.normalized;
			const key = connectionKey(draft);
			const password = draft.password;
			const typed = this.mailboxChanged || password !== '';
			const res = await connectImap({
				...(typed && { host: draft.host, port: draft.port, username: draft.username, password: password || undefined }),
				security: draft.security,
				folder: name
			});
			if (id !== this.#seq) return null;
			this.probe = res;
			if (res.ok) {
				this.folders = res.folders;
				this.#loggedIn = key;
			}
			return res;
		} catch (err) {
			if (id !== this.#seq) return null;
			this.probe = { ok: false, message: apiErrorMessage(err), detail: null, folders: [], selected: null };
			return this.probe;
		} finally {
			if (id === this.#seq) this.probing = false;
		}
	}

	/** The usual port for the draft's security, for the port field's placeholder. */
	get defaultPort(): number {
		return this.saved ? defaultPort(this.draft.security, this.saved.mailbox.defaults) : 0;
	}

	/** Another security picked: the port follows unless the user typed one of their own. */
	setSecurity(security: ImapSecurity) {
		const draft = this.draft;
		if (this.saved)
			draft.port = portAfterSecurityChange(draft.port, draft.security, security, this.saved.mailbox.defaults);
		draft.security = security;
		this.probe = null;
	}

	/** Another folder picked: examine it when the login can be tried. */
	onFolderChange(name: string) {
		this.draft.folder = name;
		this.probe = null;
		this.save.feedback = null;
		if (this.canTest) void this.checkLogin(name);
	}

	/**
	 * The setup's Test connection: log in, list the folders and examine the folder to import,
	 * the saved one for the saved mailbox, else the default (also when the saved one is gone).
	 * Nothing is saved, so nothing is imported yet. `null` when a later check superseded it.
	 */
	async testConnection(): Promise<ConnectImapResponse | null> {
		const s = this.saved;
		if (!s) return null;
		const fallback = s.mailbox.defaults.folder;
		const folder = !this.mailboxChanged && this.normalized.folder ? this.normalized.folder : fallback;
		this.draft.folder = folder;
		const res = await this.checkLogin(folder);
		if (!res?.ok || res.selected?.exists || folder === fallback) return res;
		this.draft.folder = fallback;
		return this.checkLogin(fallback);
	}

	/**
	 * The setup's save, once a test listed the folders: the connection and the folder together
	 * (the server then starts importing that folder), then the report address.
	 */
	async saveConnection(d: ImapDraft, s: SettingsResponse): Promise<SettingsResponse> {
		if (!this.loginChecked) throw new Error('Test the connection first.');
		if (this.folderStatus?.exists !== true) throw new Error('Pick a folder the server has.');
		await settings.save(saveMailbox(mailboxRequest(d, s.mailbox.poll_interval_secs)));
		const result = await settings.save(saveReporting({ report_address: this.effectiveAddress }));
		// The stored password is now the one this login used.
		this.#loggedIn = connectionKey({ ...d, password: '' });
		return result;
	}

	/** The setup's delivery step: the reporting mode. */
	async saveDelivery(d: ImapDraft): Promise<SettingsResponse> {
		return settings.save(saveReporting({ reporting_mode: d.mode }));
	}

	/**
	 * Settings: the whole mailbox, then the report address and mode, then the arrival warning
	 * when it changed. A changed connection, password or folder logs in first (and the folder
	 * must exist).
	 */
	async saveSettings(d: ImapDraft, s: SettingsResponse): Promise<{ saved: SettingsResponse; message: string }> {
		if (this.problem) throw new Error(this.problem);
		const base = fromServer(s);
		const login = this.mailboxChanged || d.security !== base.security || d.password !== '' || d.folder !== base.folder;
		if (login) {
			const res = await this.checkLogin(d.folder);
			if (!res) throw new Error('Could not connect.');
			if (!res.ok) throw new Error(res.message);
			if (!res.selected?.exists) throw new Error(`Folder "${d.folder}" was not found on the server.`);
		}
		const poll = d.pollMinutes === base.pollMinutes ? s.mailbox.poll_interval_secs : d.pollMinutes * 60;
		await settings.save(saveMailbox(mailboxRequest(d, poll)));
		let result = await settings.save(saveReporting({ report_address: this.effectiveAddress, reporting_mode: d.mode }));
		if (d.warnHours !== base.warnHours || d.stoppedDays !== base.stoppedDays) {
			result = await settings.save(
				saveMonitoring({
					report_freshness_warning_hours: d.warnHours,
					domain_reports_stopped_days: d.stoppedDays
				})
			);
			// The guide quotes the domain warning from `meta`.
			meta.reload().catch(() => {});
		}
		await systemHealth.poll();
		return { saved: result, message: login ? 'Connected and saved' : 'Saved' };
	}
}

/**
 * The Mailbox page's model over the shared settings: saved by `saveSettings` (a changed
 * connection or folder logs in first), and a saved mailbox's folders listed right away.
 */
export function mailboxSettingsModel(): ImapModel {
	const m: ImapModel = new ImapModel({
		source: { shared: settings },
		save: (d, s) => m.saveSettings(d, s),
		onLoad: (fresh, first) => {
			if (first && fresh.mailbox.configured) void m.checkLogin(fresh.mailbox.folder);
		}
	});
	return m;
}
