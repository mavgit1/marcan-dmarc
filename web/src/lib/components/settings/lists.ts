import {
	getClientReportRecipients,
	getDkimSelectors,
	getDomainMapping,
	getSenderNames,
	previewClientReportRecipients,
	previewDkimSelectors,
	previewDomainMapping,
	previewSenderNames,
	saveClientReportRecipients,
	saveDkimSelectors,
	saveDomainMapping,
	saveSenderNames
} from '$lib/api';
import { plural } from '$lib/format';
import { listTokens } from '$lib/text';
import type { ClientReportChanges } from '$lib/types/ClientReportChanges';
import type { DkimSelectorsSummary } from '$lib/types/DkimSelectorsSummary';
import type { MappingChanges } from '$lib/types/MappingChanges';
import type { MappingSummary } from '$lib/types/MappingSummary';
import type { RecipientsSummary } from '$lib/types/RecipientsSummary';
import type { SenderNameChanges } from '$lib/types/SenderNameChanges';
import type { SenderNameLine } from '$lib/types/SenderNameLine';
import type { SenderNamesSummary } from '$lib/types/SenderNamesSummary';
import type { ListSpec } from './list-draft.svelte';

/**
 * The four lists edited as text, each a `ListSpec` for `ListDraft` and `ListEditor`: the requests
 * (`ListText`: GET, POST `…/preview`, PUT) and how what saving changes is worded. The syntax of
 * each (its Format panel) is markup and lives with its page.
 */

/**
 * "4 moved, 1 added, 1 ignored (deletes 120 reports), 1 new organization"; empty when nothing
 * changes.
 */
export function describeMappingChanges(c: MappingChanges): string {
	const parts: string[] = [];
	if (c.move.length) parts.push(`${c.move.length} moved`);
	if (c.add.length) parts.push(`${c.add.length} added`);
	if (c.ignore.length) {
		const reports = c.ignore.reduce((n, i) => n + i.reports, 0);
		parts.push(`${c.ignore.length} ignored` + (reports > 0 ? ` (deletes ${plural(reports, 'report')})` : ''));
	}
	if (c.unignore.length) parts.push(`${c.unignore.length} no longer ignored (past reports are re-imported)`);
	if (c.create_orgs.length) parts.push(plural(c.create_orgs.length, 'new organization'));
	if (c.park.length) parts.push(`${c.park.length} parked`);
	if (c.unpark.length) parts.push(`${c.unpark.length} unparked`);
	return parts.join(', ');
}

/**
 * The Domains page: every domain and its organization, one `organization,domain[,status]` per
 * line; domains left out stay as they are.
 */
export const DOMAIN_LIST: ListSpec<MappingSummary> = {
	label: 'Domains',
	key: 'domain-mapping',
	get: getDomainMapping,
	preview: previewDomainMapping,
	save: saveDomainMapping,
	changes: ({ changes: c }) => {
		return {
			summary: describeMappingChanges(c),
			details: [
				{ label: 'Moved', items: c.move.map((m) => `${m.domain}: ${m.from} → ${m.to}`) },
				{ label: 'Added', items: c.add.map((a) => `${a.domain} → ${a.org}`) },
				{
					label: 'Ignored',
					items: c.ignore.map((i) => (i.reports > 0 ? `${i.domain}: deletes ${plural(i.reports, 'report')}` : i.domain))
				},
				{ label: 'No longer ignored', items: c.unignore.map((r) => `${r.domain} → ${r.org}`) },
				{ label: 'New organizations', items: c.create_orgs },
				{ label: 'Parked', items: c.park },
				{ label: 'Unparked', items: c.unpark }
			]
		};
	}
};

/** "2 added, 1 changed, 1 removed"; empty when nothing changes. */
export function describeSenderNameChanges(c: SenderNameChanges): string {
	const parts: string[] = [];
	if (c.add.length) parts.push(`${c.add.length} added`);
	if (c.change.length) parts.push(`${c.change.length} changed`);
	if (c.remove.length) parts.push(`${c.remove.length} removed`);
	return parts.join(', ');
}

/** "crm.example.com → Our CRM", or "hostpoint.ch: name hidden". */
const senderNameLine = (l: SenderNameLine) =>
	l.name === null ? `${l.target}: name hidden` : `${l.target} → ${l.name}`;

/** The Sender names page: your own names for senders, by domain, host, IP or range. */
export const SENDER_NAME_LIST: ListSpec<SenderNamesSummary> = {
	label: 'Sender names',
	key: 'sender-names',
	get: getSenderNames,
	preview: previewSenderNames,
	save: saveSenderNames,
	changes: ({ changes: c }) => ({
		summary: describeSenderNameChanges(c),
		details: [
			{ label: 'Added', items: c.add.map(senderNameLine) },
			{ label: 'Changed', items: c.change.map(senderNameLine) },
			{ label: 'Removed', items: c.remove }
		]
	})
};

/**
 * The DNS checks page's extra DKIM selectors: one per line, or separated by commas, semicolons or
 * spaces. A short list: only its line errors are shown.
 */
export const DKIM_SELECTOR_LIST: ListSpec<DkimSelectorsSummary> = {
	label: 'DKIM selectors',
	name: 'Extra DKIM selectors',
	key: 'dkim-selectors',
	get: getDkimSelectors,
	preview: previewDkimSelectors,
	save: saveDkimSelectors,
	same: (a, b) => listTokens(a).join() === listTokens(b).join(),
	placeholder: 'selector1'
};

/** "2 recipients start, 1 recipient stops"; empty when nothing changes. */
export function describeRecipientChanges(changes: ClientReportChanges): string {
	const parts: string[] = [];
	const verb = (n: number, base: string) => `${plural(n, 'recipient')} ${n === 1 ? `${base}s` : base}`;
	if (changes.start.length) parts.push(verb(changes.start.length, 'start'));
	if (changes.stop.length) parts.push(verb(changes.stop.length, 'stop'));
	return parts.join(', ');
}

/** Every change a save makes, one line each, starts first: "Starts sending to it@acme.example for Acme". */
export function recipientChangeLines(changes: ClientReportChanges): string[] {
	return [
		...changes.start.map((r) => `Starts sending to ${r.email} for ${r.organization}`),
		...changes.stop.map((r) => `Stops sending to ${r.email} for ${r.organization}`)
	];
}

/**
 * The Client reports page's recipients, `organization,email` per line. The full truth: saving
 * replaces every recipient, so the preview says who starts and who stops getting a report.
 */
export const RECIPIENT_LIST: ListSpec<RecipientsSummary> = {
	label: 'Recipients',
	name: 'Client report recipients',
	key: 'client-report-recipients',
	get: getClientReportRecipients,
	preview: previewClientReportRecipients,
	save: saveClientReportRecipients,
	same: (a, b) => a.trim() === b.trim(),
	changes: ({ changes }) => ({
		summary: describeRecipientChanges(changes),
		details: [{ items: recipientChangeLines(changes) }]
	})
};
