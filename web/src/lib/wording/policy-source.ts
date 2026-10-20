import type { PolicySource } from '$lib/types/PolicySource';
import { CHECK_FAILED } from '$lib/dns';

/**
 * Where a domain's policy comes from (`policy_source`): its DMARC record in DNS (`dns`); without
 * one, the state shown in place of the policy ("No record", the one word for a missing record).
 * Unknown is not missing. Used by `PolicyBadge` and the Enforcement tab; the server words the
 * same states in the issue badges (`wording::issues`).
 */
export const POLICY_SOURCE_TEXT: Record<PolicySource, { label?: string; title: string }> = {
	dns: { title: 'From the DMARC record in DNS' },
	not_published: { label: 'No record', title: 'No DMARC record in DNS' },
	not_checked: { label: 'Not checked yet', title: 'DNS has not been checked yet' },
	check_failed: { label: CHECK_FAILED.label, title: 'The DNS check failed; it is retried' }
};
