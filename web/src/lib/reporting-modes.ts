/**
 * The two ways reports reach the Marcan mailbox, worded once for the setup (and its guide) and
 * the Settings page (rendered by `reporting-setup/reporting-mode-cards.svelte`).
 */

import type { ReportingMode } from '$lib/types/ReportingMode';
import { emailDomain } from '$lib/report-address';

/**
 * What the two modes are a choice about, before them in the setup's Report delivery step and on
 * Settings → Mailbox: where reports go is the address in each domain's DMARC record (`rua=`).
 */
export const REPORT_DELIVERY_INTRO =
	"Receivers send DMARC reports to the address in your domain's DMARC record. Either point that address at the mailbox Marcan reads, or keep your existing address and forward the reports to it.";

/** Shown when no mailbox address is configured yet. */
export const PLACEHOLDER_ADDRESS = 'reports@example.net';

type ReportingModeCopy = {
	value: ReportingMode;
	title: string;
	recommended: boolean;
	/** One line: who this option is for. */
	fitsIf: string;
	/** Two sentences at most, with the real mailbox address. */
	explain: (address: string) => string;
	/** A tiny `rua=` example, with the real mailbox address; `forwardsTo` draws an arrow. */
	example: (address: string) => { record: string; note?: string; forwardsTo?: string };
};

export const REPORTING_MODES: readonly ReportingModeCopy[] = [
	{
		value: 'direct',
		title: 'Central mailbox',
		recommended: true,
		fitsIf: 'Every domain sends its reports straight to this address.',
		explain: (address) => {
			const domain = emailDomain(address) ?? 'the mailbox domain';
			return `Marcan checks that each DMARC record points here. Domains outside ${domain} also need one wildcard record there.`;
		},
		example: (address) => ({ record: `rua=mailto:${address}`, note: 'on every domain' })
	},
	{
		value: 'forwarded',
		title: 'Forwarded or central',
		recommended: false,
		fitsIf: 'Domains may use their own address that forwards here.',
		explain: () =>
			'Less strict: Marcan cannot see forwarding, so it accepts any report address. Mixing with central is fine.',
		example: (address) => ({ record: 'rua=mailto:dmarc@client.example', forwardsTo: address })
	}
];

export function reportingModeCopy(mode: ReportingMode): ReportingModeCopy {
	return REPORTING_MODES.find((m) => m.value === mode) ?? REPORTING_MODES[0];
}
