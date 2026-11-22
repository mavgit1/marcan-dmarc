import { createAccount, generateCertificate, testAccount, updateAccount } from '#lib/api.ts';
import { apiErrorMessage } from '#lib/http.ts';
import { looksLikeEmail } from '#lib/report-address.ts';
import { settings } from '#lib/state/settings.svelte.ts';
import { systemHealth } from '#lib/state/system-health.svelte.ts';
import type { AccountTestResponse } from '#lib/types/AccountTestResponse.ts';
import type { CertificateDraft } from '#lib/types/CertificateDraft.ts';
import type { AccountView } from '#lib/types/AccountView.ts';
import type { FolderResponse } from '#lib/types/FolderResponse.ts';
import type { ImapSecurity } from '#lib/types/ImapSecurity.ts';
import type { LoginKind } from '#lib/types/LoginKind.ts';
import type { ProviderOption } from '#lib/types/ProviderOption.ts';
import type { ProviderPreset } from '#lib/types/ProviderPreset.ts';
import type { SaveAccountRequest } from '#lib/types/SaveAccountRequest.ts';
import type { SettingsResponse } from '#lib/types/SettingsResponse.ts';
import type { SmtpSecurity } from '#lib/types/SmtpSecurity.ts';
import { defaultPort, portAfterSecurityChange, sameAccount } from './connection';
import { invalid, SettingsForm, type Invalid, type SettingsFormSource, type Writes } from './settings-form.svelte';

/**
 * The editable fields of one account, flat. A password is typed to change it; empty keeps the stored
 * one. A Microsoft login has no servers: its server fields are kept as they are on screen (another
 * provider picked before the save brings them back) but neither compared nor sent.
 */
export type AccountDraft = {
	name: string;
	/** The Provider option; it decides the login (`loginKind` follows it). */
	provider: ProviderOption;
	address: string;
	loginKind: LoginKind;
	username: string;
	password: string;
	tenantId: string;
	clientId: string;
	/** A certificate generated for this form, held by the server until the save; `null`: none (the stored one, if any, stays). */
	certificate: CertificateDraft | null;
	imapHost: string;
	imapPort: number | null;
	imapSecurity: ImapSecurity;
	smtpHost: string;
	smtpPort: number | null;
	smtpSecurity: SmtpSecurity;
	receive: boolean;
	folder: string;
	/** The other addresses whose reports are forwarded to the account, one per row as typed. */
	forwardedFrom: string[];
};

/** The server's sentences for a Microsoft login it would refuse (`wording/microsoft.rs`). */
export const MS_TEXT = {
	enterTenant: 'Enter the tenant ID.',
	enterClient: 'Enter the client ID.',
	tenant:
		"The tenant ID is a GUID: 32 hex digits in five groups. Copy the Directory (tenant) ID from the app registration's overview.",
	client:
		"The client ID is a GUID: 32 hex digits in five groups. Copy the Application (client) ID from the app registration's overview.",
	certificate: 'Generate a certificate, save, then upload it to the app registration.'
} as const;

/** The server's sentences for an account it would refuse (`wording/accounts.rs`). */
export const ACCOUNT_TEXT = {
	address: 'Enter the email address reports are sent to.',
	name: 'Enter a name for the account.',
	imapHost: "Enter the IMAP server's host name.",
	noServer: 'Enter an IMAP or SMTP server: without one the account neither receives reports nor sends.',
	username: 'Enter the username.',
	password: 'Enter the password.',
	passwordAgain: 'Enter the password again: the stored one is only used for the servers and username it was saved for.',
	serverToTest: 'Enter a server to test.'
} as const;

/** `text` has the shape of a GUID (8-4-4-4-12 hex digits), as Entra shows tenant and client IDs. */
export function isGuid(text: string): boolean {
	return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(text);
}

/** A port field's value as a number; `null` when it is empty (an input's empty value is `null` or ''). */
function portValue(port: unknown): number | null {
	if (port === null || port === undefined || port === '' || Number(port) === 0) return null;
	return Number(port);
}

/** The preset of `provider`; `undefined` when the server sent none for it. */
export function presetOf(presets: readonly ProviderPreset[], provider: ProviderOption): ProviderPreset | undefined {
	return presets.find((p) => p.provider === provider);
}

/** A new account's draft: the first preset's servers (none for Microsoft 365) and login, receiving on. */
export function newDraft(s: SettingsResponse, provider?: ProviderOption): AccountDraft {
	const preset = (provider ? presetOf(s.presets, provider) : undefined) ?? s.presets[0];
	return {
		name: '',
		provider: preset?.provider ?? 'generic',
		address: '',
		loginKind: preset?.login ?? 'password',
		username: '',
		password: '',
		tenantId: '',
		clientId: '',
		certificate: null,
		imapHost: preset?.imap?.host ?? '',
		imapPort: preset?.imap?.port ?? s.account_defaults.imap.tls,
		imapSecurity: preset?.imap?.security ?? 'tls',
		smtpHost: preset?.smtp?.host ?? '',
		smtpPort: preset?.smtp?.port ?? s.account_defaults.smtp.tls,
		smtpSecurity: preset?.smtp?.security ?? 'tls',
		receive: true,
		folder: s.account_defaults.folder,
		forwardedFrom: []
	};
}

/** The server fields of a login without servers (Microsoft), as `normalize` gives them. */
const NO_SERVERS = {
	imapHost: '',
	imapPort: null,
	imapSecurity: 'tls',
	smtpHost: '',
	smtpPort: null,
	smtpSecurity: 'tls'
} as const satisfies Partial<AccountDraft>;

/** A stored account as a draft (a Microsoft login's server fields blank). */
export function fromAccount(a: AccountView, s: SettingsResponse): AccountDraft {
	const ms = a.login.kind === 'microsoft' ? a.login : null;
	const smtp = a.smtp;
	const servers = a.imap === null && smtp === null ? NO_SERVERS : null;
	return {
		name: a.name,
		provider: a.provider,
		address: a.address,
		loginKind: a.login.kind,
		username: a.login.kind === 'password' ? a.login.username : '',
		password: '',
		tenantId: ms?.tenant_id ?? '',
		clientId: ms?.client_id ?? '',
		certificate: null,
		imapHost: a.imap?.host ?? '',
		imapPort: a.imap?.port ?? s.account_defaults.imap.tls,
		imapSecurity: a.imap?.security ?? 'tls',
		smtpHost: smtp?.host ?? '',
		// The field always shows a port: the usual one for the security when none is stored.
		smtpPort:
			smtp?.port ?? (smtp?.security === 'starttls' ? s.account_defaults.smtp.starttls : s.account_defaults.smtp.tls),
		smtpSecurity: smtp?.security ?? 'tls',
		...servers,
		receive: a.receiving.enabled,
		folder: a.receiving.folder,
		forwardedFrom: [...a.receiving.forwarded_from]
	};
}

/** The field a forwarded address's problem names: each row with that address (any case) is marked. */
export function forwardedField(address: string): string {
	return `forwardedFrom:${address.trim().toLowerCase()}`;
}

/**
 * The draft as it would be saved: trimmed, blank forwarded rows dropped, ports as numbers or `null`;
 * a Microsoft login's server fields at a fixed blank (it has none, so they never make a change).
 */
export function normalize(d: AccountDraft): AccountDraft {
	return d.loginKind === 'microsoft' ? { ...normalizeFields(d), ...NO_SERVERS } : normalizeFields(d);
}

function normalizeFields(d: AccountDraft): AccountDraft {
	return {
		...d,
		name: d.name.trim(),
		address: d.address.trim(),
		username: d.username.trim(),
		tenantId: d.tenantId.trim(),
		clientId: d.clientId.trim(),
		imapHost: d.imapHost.trim(),
		imapPort: portValue(d.imapPort),
		smtpHost: d.smtpHost.trim(),
		smtpPort: portValue(d.smtpPort),
		folder: d.folder.trim(),
		forwardedFrom: d.forwardedFrom.map((f) => f.trim()).filter((f) => f !== '')
	};
}

/** The request for a normalized draft: the password only when typed; a Microsoft login without servers. */
export function toRequest(d: AccountDraft): SaveAccountRequest {
	const receiving = { enabled: d.receive, folder: d.folder, forwarded_from: d.forwardedFrom };
	const base = { name: d.name, provider: d.provider, address: d.address, receiving };
	if (d.loginKind === 'microsoft')
		return {
			...base,
			login: { kind: 'microsoft', tenant_id: d.tenantId, client_id: d.clientId, draft_id: d.certificate?.draft_id },
			imap: null,
			smtp: null
		};
	return {
		...base,
		login: { kind: 'password', username: d.username, password: d.password || undefined },
		imap: { host: d.imapHost, port: d.imapPort ?? 0, security: d.imapSecurity },
		smtp: { host: d.smtpHost, port: d.smtpPort, security: d.smtpSecurity }
	};
}

/**
 * Whether the stored password still applies to the draft: every server it names is the saved
 * one (host case-insensitively, port) with the same username; a server removed is no change.
 * The server decides the same (`same_targets`) and refuses a save that would send it elsewhere.
 */
export function sameTargets(d: AccountDraft, saved: AccountDraft): boolean {
	const imap =
		!d.imapHost ||
		sameAccount(
			{ host: d.imapHost, port: d.imapPort, username: d.username },
			{ host: saved.imapHost, port: saved.imapPort, username: saved.username }
		);
	const smtp =
		!d.smtpHost ||
		sameAccount(
			{ host: d.smtpHost, port: d.smtpPort, username: d.username },
			{ host: saved.smtpHost, port: saved.smtpPort, username: saved.username }
		);
	return imap && smtp && d.username === saved.username;
}

/**
 * The draft with its empty ports filled in as the server fills them (`Preset::imap_port`,
 * `Preset::smtp_port`): the preset's port where the security is the preset's, else the usual one
 * for the security. An SMTP port stays empty without an SMTP host.
 */
export function withPorts(d: AccountDraft, s: SettingsResponse): AccountDraft {
	const preset = presetOf(s.presets, d.provider);
	const ports = (p: { tls: number; starttls: number }) => ({ port: p.tls, starttls_port: p.starttls });
	const imapPort =
		d.imapPort ??
		(preset?.imap && d.imapSecurity === preset.imap.security && preset.imap.port
			? preset.imap.port
			: defaultPort(d.imapSecurity, ports(s.account_defaults.imap)));
	const smtpPort =
		d.smtpPort ??
		(!d.smtpHost
			? null
			: preset?.smtp?.port && d.smtpSecurity === preset.smtp.security
				? preset.smtp.port
				: defaultPort(d.smtpSecurity, ports(s.account_defaults.smtp)));
	return { ...d, imapPort, smtpPort };
}

/**
 * What a test vouches for: everything a login and the folder use (the password only when typed).
 * A password login's test logs in to IMAP whether the account receives or not; a Microsoft login's
 * reads its mailbox (the address) only while it receives.
 */
export function loginKey(d: AccountDraft): string {
	if (d.loginKind === 'microsoft')
		return JSON.stringify([
			d.provider,
			d.loginKind,
			d.address.toLowerCase(),
			d.tenantId,
			d.clientId,
			d.certificate?.draft_id ?? null,
			d.receive
		]);
	return JSON.stringify([
		d.provider,
		d.loginKind,
		d.username,
		d.password,
		d.imapHost.toLowerCase(),
		d.imapPort,
		d.imapSecurity,
		d.smtpHost.toLowerCase(),
		d.smtpPort,
		d.smtpSecurity
	]);
}

export type AccountModelOptions = {
	/** The shared settings (Settings) or handed in by the parent (a setup step). */
	source: SettingsFormSource<SettingsResponse>;
	/** The stored account it edits; `null` for a new one. */
	id: number | null;
	/** A new account was stored: its id. */
	onCreated?: (id: number, saved: SettingsResponse) => void;
};

/**
 * One account's form (the Account page, the setup's account step): its draft over the stored
 * account (or a new one from the first preset), the preset a provider brings, the username that
 * follows the address until typed, when the password is needed, the test (token, mailbox and
 * folders, SMTP login; nothing stored) and the save, which tests a changed login first and
 * answers every setting. Create it during component initialisation (it owns effects).
 */
export class AccountModel extends SettingsForm<SettingsResponse, AccountDraft> {
	static readonly writes: Writes = { page: 'accounts', resource: 'accounts' };
	/** The stored account it edits; `null` until a new one is saved. */
	id = $state<number | null>(null);
	/** The last test's answer, and the login it tried (`loginKey`). */
	result = $state.raw<AccountTestResponse | null>(null);
	#testedKey = $state<string | null>(null);
	testing = $state(false);
	#seq = 0;
	/** Folders the last successful mailbox step listed. */
	folders = $state.raw<FolderResponse[] | null>(null);
	/**
	 * A new account's provider was picked (`setProvider`): until then its form offers only the
	 * choice. A stored account's provider is fixed: another one is another account.
	 */
	#providerPicked = $state(false);
	get providerChosen(): boolean {
		return this.id !== null || this.#providerPicked;
	}

	/** The stored account; `null` for a new one (or one deleted meanwhile). */
	account: AccountView | null = $derived.by(
		() => (this.id === null ? null : this.saved?.accounts.find((a) => a.id === this.id)) ?? null
	);
	preset: ProviderPreset | undefined = $derived.by(() =>
		this.saved ? presetOf(this.saved.presets, this.draft.provider) : undefined
	);
	/** The stored password applies to the servers and username on screen. */
	passwordStored: boolean = $derived.by(() => {
		const a = this.account;
		if (!a || a.login.kind !== 'password' || !a.login.has_password || !this.base) return false;
		const s = this.saved;
		if (!s || this.base.loginKind !== 'password') return false;
		return sameTargets(withPorts(this.normalized, s), withPorts(this.base, s));
	});
	/** A password is stored for this account, whatever its servers. */
	passwordSaved: boolean = $derived.by(() => {
		const a = this.account;
		return a?.login.kind === 'password' && a.login.has_password;
	});
	/** The account has a server, so its login needs a password (typed, or the stored one that applies). */
	passwordRequired: boolean = $derived.by(() => {
		const d = this.normalized;
		return d.loginKind === 'password' && (!!d.imapHost || !!d.smtpHost) && !this.passwordStored;
	});
	/** The stored certificate (it stays whatever IDs it is used for); `null`: none. */
	storedCertificate = $derived.by(() => {
		const a = this.account;
		return a?.login.kind === 'microsoft' ? a.login.certificate : null;
	});
	/** Why the Microsoft login on screen cannot be tried or saved, with the field it concerns. */
	microsoftProblem: Invalid | null = $derived.by(() => {
		const d = this.normalized;
		if (d.loginKind !== 'microsoft') return null;
		if (!d.tenantId) return invalid('tenantId', MS_TEXT.enterTenant);
		if (!isGuid(d.tenantId)) return invalid('tenantId', MS_TEXT.tenant);
		if (!d.clientId) return invalid('clientId', MS_TEXT.enterClient);
		if (!isGuid(d.clientId)) return invalid('clientId', MS_TEXT.client);
		if (!d.certificate && !this.storedCertificate) return invalid('certificate', MS_TEXT.certificate);
		return null;
	});
	/** Why the password field holds the save: none typed where one is needed; the stored one is for other servers. */
	#passwordProblem: Invalid | null = $derived.by(() => {
		if (!this.passwordRequired || this.normalized.password.trim()) return null;
		return invalid('password', this.passwordSaved ? ACCOUNT_TEXT.passwordAgain : ACCOUNT_TEXT.password);
	});
	/**
	 * A credential is stored for the login on screen, or one is typed or generated: what the line
	 * on storing it unencrypted is about.
	 */
	credentialPresent: boolean = $derived.by(() => {
		const d = this.draft;
		if (d.loginKind === 'password') return this.passwordSaved || d.password !== '';
		return !!d.certificate || !!this.storedCertificate;
	});
	/** A field of the form was left (`touch`). */
	#touched = $state(false);

	/** The user left a field of the form (its `focusout`): a new account's entries count from now on. */
	touch(): void {
		this.#touched = true;
	}

	/** The fields typed in and left since the last save or discard (`leave`). */
	#left = $state.raw<ReadonlySet<string>>(new Set());
	/** Save was pressed with a problem not said yet (`reveal`). */
	#revealed = $state(false);

	/** The user typed in `field` and left it: a problem about it is said from now on. */
	leave(field: string): void {
		if (!this.#left.has(field)) this.#left = new Set([...this.#left, field]);
	}

	override reveal(): void {
		this.#revealed = true;
	}

	/**
	 * A problem is said under its field (and beside Save) once the field was typed in and left,
	 * or Save was pressed: never about a field the user has not come to yet. A problem about
	 * something without a field to type in (the certificate to generate) is said at once, and so
	 * is a stored password that no longer applies.
	 */
	protected override shows(field: string): boolean {
		if (this.#revealed || this.#left.has(field)) return true;
		// A stored password an edit just put out of use (other servers): its field emptied on
		// screen, so why is said at once.
		if (field === 'password' && this.passwordSaved && !this.passwordStored) return true;
		return !TYPED.has(field) && !field.startsWith('forwardedFrom:');
	}

	#quiet(): void {
		this.#left = new Set();
		this.#revealed = false;
	}

	/** Back to the saved account; a new one back to the choice of its provider. */
	override discard(): void {
		super.discard();
		this.#providerPicked = false;
		this.#quiet();
	}

	/**
	 * Unsaved changes. A stored account's: every change. A new account's form has none until the
	 * user left one of its fields and something differs from what its provider starts with: what
	 * the browser fills in by itself (a saved login autofilled) and the servers a picked provider
	 * fills in show no save bar, mark no field and say no problem; Save waits meanwhile.
	 */
	override get dirty(): boolean {
		if (this.id !== null) return super.dirty;
		if (!this.#touched) return false;
		const s = this.saved;
		if (!s) return super.dirty;
		return this.isDirty(this.normalized, normalize(newDraft(s, this.draft.provider)));
	}

	/** Something was edited, so the held field is marked. */
	edited: boolean = $derived.by(() => this.dirty);
	/**
	 * Changed since saved in a way only a login can vouch for: the servers, the login, the folder,
	 * receiving switched on. Switching it off is not: a mailbox whose login fails can be stopped.
	 */
	loginChanged: boolean = $derived.by(() => {
		const base = this.base;
		if (!base || this.account === null) return true;
		const d = this.normalized;
		return loginKey(d) !== loginKey(base) || d.folder !== base.folder || (d.receive && !base.receive);
	});
	/** The last test tried the login on screen. */
	tested: boolean = $derived.by(() => this.#testedKey !== null && this.#testedKey === loginKey(this.normalized));
	/** The examined folder's state, once a test answered for the folder now chosen. */
	folderStatus = $derived.by(() => {
		const sel = this.tested ? this.result?.mailbox?.selected : null;
		return sel && sel.name === this.normalized.folder ? sel : null;
	});

	#onCreated: AccountModelOptions['onCreated'];

	constructor(options: AccountModelOptions) {
		super({ ...options.source, label: 'Account', blank: BLANK });
		this.id = options.id;
		this.#onCreated = options.onCreated;
	}

	protected toDraft(s: SettingsResponse): AccountDraft {
		const a = this.id === null ? null : s.accounts.find((x) => x.id === this.id);
		return a ? fromAccount(a, s) : newDraft(s);
	}

	protected normalize(d: AccountDraft): AccountDraft {
		return normalize(d);
	}

	/** A typed password is a change by itself and never compared. */
	protected isDirty(draft: AccountDraft, base: AccountDraft): boolean {
		if (draft.password !== '') return true;
		return JSON.stringify({ ...draft, password: '' }) !== JSON.stringify(base);
	}

	/**
	 * What the server would refuse, worded like it (its 422s, and the 409 of an address another
	 * account uses): client-side because a save starts with a test login, which this holds until
	 * the account is complete enough to try.
	 */
	protected check(d: AccountDraft): Invalid | null {
		if (d.address && !looksLikeEmail(d.address))
			return invalid('address', `The email address ${d.address} is not a valid email address.`);
		if (d.receive && !d.address) return invalid('address', ACCOUNT_TEXT.address);
		if (!d.name && !d.address) return invalid('name', ACCOUNT_TEXT.name);
		/** Another account has `address` as its own or a forwarded one. */
		const taken = (address: string) =>
			this.saved?.accounts.some(
				(a) =>
					a.id !== this.id &&
					[a.address, ...a.receiving.forwarded_from].some((o) => o !== '' && o.toLowerCase() === address.toLowerCase())
			);
		if (d.address && taken(d.address)) return invalid('address', `Another account already uses ${d.address}.`);
		const seen = new Set<string>();
		for (const f of d.forwardedFrom) {
			const field = forwardedField(f);
			if (!looksLikeEmail(f)) return invalid(field, `The forwarding address ${f} is not a valid email address.`);
			if (f.toLowerCase() === d.address.toLowerCase())
				return invalid(field, `The forwarding address ${f} is the account's own address.`);
			if (seen.has(f.toLowerCase())) return invalid(field, `The forwarding address ${f} is entered twice.`);
			seen.add(f.toLowerCase());
			if (taken(f)) return invalid(field, `Another account already uses ${f}.`);
		}
		// A Microsoft login reads and sends over Graph: it has no servers.
		if (d.loginKind === 'microsoft') return this.microsoftProblem;
		if (d.receive && !d.imapHost) return invalid('imapHost', ACCOUNT_TEXT.imapHost);
		if (!d.imapHost && !d.smtpHost) return invalid('imapHost', ACCOUNT_TEXT.noServer);
		if (!d.username) return invalid('username', ACCOUNT_TEXT.username);
		return this.#passwordProblem;
	}

	/** The held field is marked only once something is `edited`. */
	ariaInvalid(field: string): true | undefined {
		return (this.edited && this.fieldProblem(field) !== null) || undefined;
	}

	/** Why the test cannot run now, with the field it concerns (`''`: none); `null` when it can. */
	get testHold(): Invalid | null {
		const d = this.normalized;
		if (d.loginKind === 'microsoft') return this.microsoftProblem;
		if (!d.imapHost && !d.smtpHost) return invalid('', ACCOUNT_TEXT.serverToTest);
		if (!d.username) return invalid('username', ACCOUNT_TEXT.username);
		return this.#passwordProblem;
	}

	/** Why the test cannot run now; `null` when it can. */
	get testProblem(): string | null {
		return this.testHold?.message ?? null;
	}

	/**
	 * What the Test row says while the test cannot run: its reason, unless the form already says it
	 * under its field (the save bar says it too), so a missing password is not said three times.
	 */
	get testNote(): string | null {
		const hold = this.testHold;
		if (!hold) return null;
		if (hold.field && this.edited && this.fieldProblem(hold.field) === hold.message) return null;
		return hold.message;
	}

	/**
	 * Generate a key pair and certificate for this form (named after its address, else its name):
	 * the server keeps both until the save stores them with the account; then the certificate is
	 * uploaded to the app registration and the login tested.
	 */
	async generateCertificate(): Promise<CertificateDraft> {
		const d = this.normalized;
		const cert = await generateCertificate({ subject: d.address || d.name });
		this.draft.certificate = cert;
		this.save.feedback = null;
		return cert;
	}

	/**
	 * Another provider picked: its login, and its servers unless they were typed by hand (one
	 * without servers, Microsoft 365, empties them the same way; it neither shows nor sends them).
	 */
	setProvider(provider: ProviderOption) {
		const s = this.saved;
		const draft = this.draft;
		const before = s ? presetOf(s.presets, draft.provider) : undefined;
		const next = s ? presetOf(s.presets, provider) : undefined;
		draft.provider = provider;
		this.#providerPicked = true;
		if (!next) return;
		draft.loginKind = next.login;
		const untouched = (host: string, preset: string | undefined) => !host.trim() || host.trim() === preset;
		const defaults = s?.account_defaults;
		const imap = next.imap ?? { host: '', port: defaults?.imap.tls ?? null, security: 'tls' as const };
		const smtp = next.smtp ?? { host: '', port: null, security: 'tls' as const };
		if (untouched(draft.imapHost, before?.imap?.host)) {
			draft.imapHost = imap.host;
			draft.imapPort = imap.port;
			draft.imapSecurity = imap.security;
		}
		if (untouched(draft.smtpHost, before?.smtp?.host)) {
			draft.smtpHost = smtp.host;
			draft.smtpPort = smtp.port ?? defaults?.smtp.tls ?? null;
			draft.smtpSecurity = smtp.security;
		}
	}

	/** The address typed: the username follows it while it is empty or still the old address. */
	setAddress(address: string) {
		const draft = this.draft;
		if (draft.username === '' || draft.username === draft.address) draft.username = address;
		draft.address = address;
	}

	/** Another IMAP security picked: an untouched port follows. */
	setImapSecurity(security: ImapSecurity) {
		const s = this.saved;
		const draft = this.draft;
		if (s) {
			const ports = { port: s.account_defaults.imap.tls, starttls_port: s.account_defaults.imap.starttls };
			draft.imapPort = portAfterSecurityChange(draft.imapPort, draft.imapSecurity, security, ports);
		}
		draft.imapSecurity = security;
	}

	/** Another SMTP security picked: an untouched port follows. */
	setSmtpSecurity(security: SmtpSecurity) {
		const s = this.saved;
		const draft = this.draft;
		if (s) {
			const ports = { port: s.account_defaults.smtp.tls, starttls_port: s.account_defaults.smtp.starttls };
			draft.smtpPort = portAfterSecurityChange(draft.smtpPort, draft.smtpSecurity, security, ports);
		}
		draft.smtpSecurity = security;
	}

	/**
	 * Test the account on screen: a password login's IMAP login with its folders (examining
	 * `folder`, default the draft's) and its SMTP login; a Microsoft login's token, then (while it
	 * receives) its mailbox read over Graph with its folders; it sends over Graph, proven by
	 * Settings → Sending's test message. Nothing is stored. `null` when a later test superseded
	 * this one.
	 */
	async testLogin(folder?: string): Promise<AccountTestResponse | null> {
		const seq = ++this.#seq;
		const d = this.normalized;
		const key = loginKey(d);
		this.testing = true;
		try {
			const res = await testAccount({
				id: this.account ? this.account.id : undefined,
				account: toRequest(d),
				folder: folder ?? d.folder,
				// A Microsoft login has no SMTP line: it sends over Graph.
				smtp: d.loginKind === 'password' && !!d.smtpHost
			});
			if (seq !== this.#seq) return null;
			this.result = res;
			this.#testedKey = key;
			if (res.mailbox?.ok) this.folders = res.mailbox.folders;
			return res;
		} catch (error) {
			if (seq !== this.#seq) return null;
			const message = apiErrorMessage(error);
			this.result = {
				token: null,
				mailbox: { ok: false, message, detail: null, docs: null, folders: [], selected: null },
				smtp: null
			};
			this.#testedKey = key;
			return this.result;
		} finally {
			if (seq === this.#seq) this.testing = false;
		}
	}

	/** Another folder picked: examined at once when the login can be tried. */
	pickFolder(name: string) {
		this.draft.folder = name;
		this.save.feedback = null;
		if (this.testProblem === null) void this.testLogin(name);
	}

	/** Why the last test says the account cannot work as it is; `null` when every step passed. */
	static failure(res: AccountTestResponse, d: AccountDraft): string | null {
		for (const step of [res.token, res.mailbox, res.smtp]) if (step && !step.ok) return step.message;
		if (d.receive && res.mailbox?.selected && !res.mailbox.selected.exists)
			return `Folder "${res.mailbox.selected.name}" was not found on the server.`;
		return null;
	}

	/**
	 * Save the account: a changed login is tested first (every step must pass and the folder must
	 * exist), then the whole account goes to the server, which checks it again and answers every
	 * setting. A newly generated certificate is saved untested: the app registration accepts it
	 * only once it is uploaded, which comes after the save.
	 */
	protected async write(d: AccountDraft): Promise<{ saved: SettingsResponse; message?: string }> {
		let message: string | undefined;
		const newCertificate = d.loginKind === 'microsoft' && !!d.certificate;
		if (newCertificate) {
			message = 'Saved: upload the certificate to the app registration, then test';
		} else if (this.loginChanged) {
			const res = await this.testLogin(d.folder);
			if (!res) throw new Error('The test did not finish.');
			const failed = AccountModel.failure(res, d);
			if (failed) throw new Error(failed);
			message = 'Tested and saved';
		}
		const before = new Set(this.saved?.accounts.map((a) => a.id) ?? []);
		const id = this.account?.id ?? null;
		const saved = await settings.save(id === null ? createAccount(toRequest(d)) : updateAccount(id, toRequest(d)));
		if (id === null) {
			const created = saved.accounts.find((a) => !before.has(a.id));
			if (created) {
				this.id = created.id;
				this.#onCreated?.(created.id, saved);
			}
		}
		void systemHealth.poll();
		this.#quiet();
		return { saved, message };
	}
}

/** The fields with a control to type in: a problem about one waits until it was left (`shows`). */
const TYPED = new Set([
	'name',
	'address',
	'username',
	'password',
	'tenantId',
	'clientId',
	'imapHost',
	'imapPort',
	'smtpPort'
]);

const BLANK: AccountDraft = {
	name: '',
	provider: 'generic',
	address: '',
	loginKind: 'password',
	username: '',
	password: '',
	tenantId: '',
	clientId: '',
	certificate: null,
	imapHost: '',
	imapPort: null,
	imapSecurity: 'tls',
	smtpHost: '',
	smtpPort: null,
	smtpSecurity: 'tls',
	receive: true,
	folder: '',
	forwardedFrom: []
};
