import type { ImapSecurity } from '$lib/types/ImapSecurity';
import type { SmtpSecurity } from '$lib/types/SmtpSecurity';

/**
 * What the mailbox (IMAP) and the sending account (SMTP) share: a host, a port, a login and the
 * TLS / STARTTLS choice, whose usual ports are the group's `defaults`. One rule for both.
 */

/** The TLS / STARTTLS choice of either account (the same two values on the server). */
export type Security = ImapSecurity | SmtpSecurity;

/** The choices of the security select; the port field beside it shows the number. */
export const SECURITY_LABEL: Record<Security, string> = {
	tls: 'TLS',
	starttls: 'STARTTLS'
};

/** The usual ports of an account (`MailboxDefaults`, `SendingDefaults`). */
export type PortDefaults = { port: number; starttls_port: number };

/** The usual port for `security`. */
export function defaultPort(security: Security, defaults: PortDefaults): number {
	return security === 'starttls' ? defaults.starttls_port : defaults.port;
}

/**
 * The port after switching from `from` to `to`: the usual port of `to` while the field is empty
 * (nothing, or 0) or still holds the usual port of `from`, else the port the user typed.
 */
export function portAfterSecurityChange<P extends number | null>(
	port: P,
	from: Security,
	to: Security,
	defaults: PortDefaults
): number | P {
	const value = port === null || (port as unknown) === '' ? 0 : Number(port);
	const untouched = !(value > 0) || value === defaultPort(from, defaults);
	return untouched ? defaultPort(to, defaults) : port;
}

/** An account's login: where it connects and as whom. */
export type Login = { host: string; port: number | null; username: string };

/** Same host (case-insensitively), port and username: the saved password applies. */
export function sameAccount(a: Login, b: Login): boolean {
	return a.host.toLowerCase() === b.host.toLowerCase() && a.port === b.port && a.username === b.username;
}
