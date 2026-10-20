import type { IdentitySource } from '$lib/types/IdentitySource';
import type { SenderIdentity } from '$lib/types/SenderIdentity';

/** Fallback when no evidence names the sender. */
const UNKNOWN_SENDER_NAME = 'Unknown';

/** Title / tooltip for an operator named only by reverse DNS that failed forward confirmation. */
export const UNCONFIRMED_PTR_HINT = 'Reverse DNS is not forward-confirmed';

/**
 * Caption for a sending IP: the operator's name, else its domain, else Unknown. Neutral
 * caption only - never feed this into a verdict, tint, or sort key.
 */
export function displayName(identity: SenderIdentity | null | undefined): string {
	return identity?.name ?? identity?.operator_domain ?? UNKNOWN_SENDER_NAME;
}

/**
 * True when the caption rests on reverse DNS that failed forward confirmation. Still show
 * the name; only mark confidence in the name itself, not the sender's behaviour.
 */
export function isUnconfirmedOperatorName(identity: SenderIdentity | null | undefined): boolean {
	return identity?.source === 'ptr' && identity.ptr_confirmed === false;
}

/** Where a sender's caption comes from, in words: "Forward-confirmed reverse DNS: mail.example.net", …. */
export const IDENTITY_EVIDENCE: Record<IdentitySource, (identity: SenderIdentity) => string> = {
	ptr: (identity) =>
		identity.ptr_confirmed
			? `Forward-confirmed reverse DNS: ${identity.ptr_hostname}`
			: `Reverse DNS (not forward-confirmed): ${identity.ptr_hostname}`,
	spf: (identity) => `SPF passed for ${identity.spf_domain} in reports`
};

/** {@link IDENTITY_EVIDENCE} for a caption; `null` without evidence. */
export function identityEvidence(identity: SenderIdentity | null | undefined): string | null {
	return identity?.source ? IDENTITY_EVIDENCE[identity.source](identity) : null;
}
