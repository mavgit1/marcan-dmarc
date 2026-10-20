// The request functions of the JSON API, one per endpoint. Response and request types are
// generated from the Rust API by ts-rs into `$lib/types` (`cargo test` writes them; never edit
// them by hand) and imported from there, here and at every call site. The client, the query
// serializer and the error classifiers live in `$lib/http`.
import { api, searchParams } from '$lib/http';
import type { DkimSelectorsSummary } from '$lib/types/DkimSelectorsSummary';
import type { ListText } from '$lib/types/ListText';
import type { MappingSummary } from '$lib/types/MappingSummary';
import type { RecipientsSummary } from '$lib/types/RecipientsSummary';
import type { SendClientReportEmailRequest } from '$lib/types/SendClientReportEmailRequest';
import type { SenderNamesSummary } from '$lib/types/SenderNamesSummary';
import type { AlertSettings } from '$lib/types/AlertSettings';
import type { BrandingSettings } from '$lib/types/BrandingSettings';
import type { ClientReportSettings } from '$lib/types/ClientReportSettings';
import type { DnsSettings } from '$lib/types/DnsSettings';
import type { AlertItem } from '$lib/types/AlertItem';
import type { BimiCheckResult } from '$lib/types/BimiCheckResult';
import type { ConnectImapRequest } from '$lib/types/ConnectImapRequest';
import type { ConnectImapResponse } from '$lib/types/ConnectImapResponse';
import type { ClientReportEmail } from '$lib/types/ClientReportEmail';
import type { BrandingPreviewRequest } from '$lib/types/BrandingPreviewRequest';
import type { CreateDomainBody } from '$lib/types/CreateDomainBody';
import type { CreatedDomain } from '$lib/types/CreatedDomain';
import type { DkimCheckResult } from '$lib/types/DkimCheckResult';
import type { DmarcCheckResult } from '$lib/types/DmarcCheckResult';
import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
import type { DomainFailureRow } from '$lib/types/DomainFailureRow';
import type { DomainOverview } from '$lib/types/DomainOverview';
import type { DomainSummary } from '$lib/types/DomainSummary';
import type { EmailPreview } from '$lib/types/EmailPreview';
import type { FailureOutcome } from '$lib/types/FailureOutcome';
import type { Items } from '$lib/types/Items';
import type { Meta } from '$lib/types/Meta';
import type { MonitoringSettings } from '$lib/types/MonitoringSettings';
import type { MtaStsCheckResult } from '$lib/types/MtaStsCheckResult';
import type { OnboardingSettings } from '$lib/types/OnboardingSettings';
import type { OnboardingState } from '$lib/types/OnboardingState';
import type { OrganizationNameRequest } from '$lib/types/OrganizationNameRequest';
import type { OrganizationRow } from '$lib/types/OrganizationRow';
import type { OrganizationSummary } from '$lib/types/OrganizationSummary';
import type { Page } from '$lib/types/Page';
import type { PtrLookup } from '$lib/types/PtrLookup';
import type { ReportDetailResponse } from '$lib/types/ReportDetailResponse';
import type { ReportSummary } from '$lib/types/ReportSummary';
import type { RetentionSettings } from '$lib/types/RetentionSettings';
import type { ReportingSetupResponse } from '$lib/types/ReportingSetupResponse';
import type { SaveLicenseRequest } from '$lib/types/SaveLicenseRequest';
import type { SaveMailboxRequest } from '$lib/types/SaveMailboxRequest';
import type { SaveReportingRequest } from '$lib/types/SaveReportingRequest';
import type { SaveSendingRequest } from '$lib/types/SaveSendingRequest';
import type { ScopeOptions } from '$lib/types/ScopeOptions';
import type { SendTestAlertRequest } from '$lib/types/SendTestAlertRequest';
import type { SendTestRequest } from '$lib/types/SendTestRequest';
import type { SendTestResponse } from '$lib/types/SendTestResponse';
import type { SenderDetailResponse } from '$lib/types/SenderDetailResponse';
import type { SenderSummary } from '$lib/types/SenderSummary';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import type { SpfCheckResult } from '$lib/types/SpfCheckResult';
import type { Statistics } from '$lib/types/Statistics';
import type { SyncImapResponse } from '$lib/types/SyncImapResponse';
import type { SystemHealthResponse } from '$lib/types/SystemHealthResponse';
import type { UpcomingClientReport } from '$lib/types/UpcomingClientReport';
import type { TimeseriesPoint } from '$lib/types/TimeseriesPoint';
import type { TlsRptCheckResult } from '$lib/types/TlsRptCheckResult';
import type { UpdateDomainBody } from '$lib/types/UpdateDomainBody';
import type { WithIdentity } from '$lib/types/WithIdentity';

/** Optional organization / domain / since filters shared by several endpoints. */
export type ScopeParams = {
	organization_id?: number | null;
	domain_id?: number | null;
	since?: string | null;
};

/**
 * Paging and server-side order of a list. `sort` is a column key, `-key` for descending;
 * `limit` must be 1–500 (default 50) and `offset` non-negative.
 */
export type PageParams = {
	limit?: number;
	offset?: number;
	sort?: string | null;
};

/** `failing`: only senders with mail that failed DMARC and was not forwarded. */
export type SendersParams = ScopeParams & PageParams & { failing?: boolean };

/** The alert list: scope (`since` is over when a change was found) and paging, newest first. */
export type AlertsParams = ScopeParams & Omit<PageParams, 'sort'>;

/** `q` searches domain and reporting organization on the server. */
export type ReportsParams = ScopeParams & PageParams & { q?: string | null };

/** `q` filters by domain or organization name on the server. */
export type DomainsParams = ScopeParams & PageParams & { q?: string | null };

export type TimeseriesParams = ScopeParams & {
	bucket?: 'day' | 'week';
};

/** A live DNS check can take up to ~45 s on a slow resolver (the server then answers with the last good result). */
const DNS_LIVE_TIMEOUT = 60_000;

export function getTimeseries(params?: TimeseriesParams): Promise<Items<TimeseriesPoint>> {
	return api.get('timeseries', { searchParams: searchParams(params) }).json();
}

/** Organizations with their domains' figures and attention summary within `since`. */
export function getOrganizations(params?: { since?: string | null }): Promise<Items<OrganizationRow>> {
	return api.get('organizations', { searchParams: searchParams(params) }).json();
}

/** The rules' fixed numbers (99 / 95, score points, grade bands), for help text. */
export function getMeta(): Promise<Meta> {
	return api.get('meta').json();
}

/** 409 when the name is taken. */
export function createOrganization(body: OrganizationNameRequest): Promise<OrganizationSummary> {
	return api.post('organizations', { json: body }).json();
}

/** Rename; 409 when the name is taken. */
export function updateOrganization(id: number, body: OrganizationNameRequest): Promise<OrganizationSummary> {
	return api.patch(`organizations/${id}`, { json: body }).json();
}

/** Delete a non-default organization; its domains and reports move to the default one. */
export async function deleteOrganization(id: number): Promise<void> {
	await api.delete(`organizations/${id}`);
}

/** Identity only; figures come from `getDomains` / `getStatistics` scoped to the organization. */
export function getOrganization(id: number): Promise<OrganizationSummary> {
	return api.get(`organizations/${id}`).json();
}

/** The organization's next client report; `null` when it gets none (reports or sending off, not listed). */
export function getUpcomingClientReport(id: number): Promise<UpcomingClientReport | null> {
	return api.get(`organizations/${id}/client-report`).json();
}

/** Organizations and monitored domains, ids and names only (the header's scope filter). */
export function getScope(): Promise<ScopeOptions> {
	return api.get('scope').json();
}

/** One page of the domain list; `q` filters by domain or organization name on the server. */
export function getDomains(params?: DomainsParams): Promise<Page<DomainSummary>> {
	return api.get('domains', { searchParams: searchParams(params) }).json();
}

/**
 * Every domain in the scope with its figures, by name, for a view that needs them all at once
 * (the dashboard's list while the scope has no report data): the domain list's pages at the
 * largest page size, until its total is reached. Names alone come from `getScope`.
 */
export async function getAllDomains(params?: ScopeParams): Promise<DomainSummary[]> {
	const limit = 500;
	const all: DomainSummary[] = [];
	for (;;) {
		const page = await getDomains({ ...params, limit, offset: all.length });
		all.push(...page.items);
		if (page.items.length === 0 || all.length >= page.total) return all;
	}
}

/** The scope's domains with an open issue, the first `limit` in their attention rank over the whole scope, and `total`. */
export function getAttentionDomains(params?: ScopeParams & { limit?: number }): Promise<Page<DomainSummary>> {
	return api.get('domains/attention', { searchParams: searchParams(params) }).json();
}

/** `unignored` is true when the name belonged to an ignored domain, which is
 * monitored again (in `organization_id`); a mailbox re-import brings its past reports back. */
export function createDomain(body: CreateDomainBody): Promise<CreatedDomain> {
	return api.post('domains', { json: body }).json();
}

/** Move a domain to another organization and/or park it; send at least one field. */
export function updateDomain(id: number, body: UpdateDomainBody): Promise<DomainSummary> {
	return api.patch(`domains/${id}`, { json: body }).json();
}

/**
 * Ignore a domain (`DELETE /api/domains/{id}`): its reports are deleted and new ones are
 * dropped. Undone by taking `ignored` off its line in the domain mapping.
 */
export async function ignoreDomain(id: number): Promise<void> {
	await api.delete(`domains/${id}`);
}

export function getDomain(id: number, params?: { since?: string | null }): Promise<DomainOverview> {
	return api.get(`domains/${id}`, { searchParams: searchParams(params) }).json();
}

/** The full live check of a monitored domain; `refresh` bypasses the server's caches. */
export function getDnsCheck(domain: string, params?: { refresh?: boolean }): Promise<DnsCheckResponse> {
	return api
		.get('dns/check', {
			searchParams: searchParams({ domain, refresh: params?.refresh }),
			timeout: DNS_LIVE_TIMEOUT
		})
		.json();
}

/** `refresh` bypasses the server's caches (a user-initiated re-check). */
export function getDnsDmarc(domain: string, params?: { refresh?: boolean }): Promise<DmarcCheckResult> {
	return api
		.get('dns/dmarc', {
			searchParams: searchParams({ domain, refresh: params?.refresh }),
			timeout: DNS_LIVE_TIMEOUT
		})
		.json();
}

/**
 * How the monitored domains' reports reach the mailbox, each domain's status decided by the
 * server from live lookups. `domain_id` asks for one domain (its record looked up even while
 * its reports arrive); `refresh` bypasses the server's DNS caches (Re-check).
 */
export function getReportingSetup(params?: { domain_id?: number; refresh?: boolean }): Promise<ReportingSetupResponse> {
	return api
		.get('reporting-setup', {
			searchParams: searchParams({ domain_id: params?.domain_id, refresh: params?.refresh }),
			timeout: DNS_LIVE_TIMEOUT
		})
		.json();
}

export function getDnsSpf(domain: string): Promise<SpfCheckResult> {
	return api.get('dns/spf', { searchParams: { domain }, timeout: DNS_LIVE_TIMEOUT }).json();
}

/** When `selectors` is set, the server probes exactly those names (pinned). */
export function getDnsDkim(domain: string, selectors?: string[]): Promise<DkimCheckResult> {
	return api
		.get('dns/dkim', {
			searchParams: searchParams({ domain, selectors: selectors?.join(',') }),
			timeout: DNS_LIVE_TIMEOUT
		})
		.json();
}

export function getDnsMtaSts(domain: string): Promise<MtaStsCheckResult> {
	return api.get('dns/mta-sts', { searchParams: { domain }, timeout: DNS_LIVE_TIMEOUT }).json();
}

export function getDnsTlsRpt(domain: string): Promise<TlsRptCheckResult> {
	return api.get('dns/tls-rpt', { searchParams: { domain }, timeout: DNS_LIVE_TIMEOUT }).json();
}

export function getDnsBimi(domain: string): Promise<BimiCheckResult> {
	return api.get('dns/bimi', { searchParams: { domain }, timeout: DNS_LIVE_TIMEOUT }).json();
}

export function getDnsPtr(ip: string): Promise<PtrLookup> {
	return api.get('dns/ptr', { searchParams: { ip }, timeout: DNS_LIVE_TIMEOUT }).json();
}

/** What changed, newest first. */
export function getAlerts(params?: AlertsParams): Promise<Page<AlertItem>> {
	return api.get('alerts', { searchParams: searchParams(params) }).json();
}

export function getReports(params?: ReportsParams): Promise<Page<ReportSummary>> {
	return api.get('reports', { searchParams: searchParams(params) }).json();
}

export function getReport(id: number): Promise<ReportDetailResponse> {
	return api.get(`reports/${id}`).json();
}

/** Source IPs in a scope, sorted and paged on the server (default: busiest first). */
export function getSenders(params?: SendersParams): Promise<Page<WithIdentity<SenderSummary>>> {
	return api.get('senders', { searchParams: searchParams(params) }).json();
}

/** IPv6 must be encoded — 400 if not an IP, 404 if unseen in scope. */
export function getSender(ip: string, params?: ScopeParams): Promise<SenderDetailResponse> {
	return api.get(`senders/${encodeURIComponent(ip)}`, { searchParams: searchParams(params) }).json();
}

export function getStatistics(params?: ScopeParams): Promise<Statistics> {
	return api.get('statistics', { searchParams: searchParams(params) }).json();
}

/** Filters of the Failures tab; its CSV export takes the same ones. */
export type DomainFailureFilters = {
	since?: string | null;
	/** The kinds to include, a record matching any of them; null or empty: every record. */
	outcome?: readonly FailureOutcome[] | null;
	source_ip?: string | null;
};

/** `outcome` as the server reads it: the kinds separated by commas, none for every record. */
export function outcomeParam(outcome: readonly FailureOutcome[] | null | undefined): string | null {
	return outcome?.length ? outcome.join(',') : null;
}

/**
 * The failures behind `delivered_failures`: DMARC failed (neither SPF nor DKIM passed
 * aligned), the receiver delivered the mail anyway, and it was not a trusted forward.
 * Link to them with `?tab=failures&outcome=delivered`.
 */
export const DELIVERED_FAILURE_FILTER = {
	outcome: ['delivered']
} as const satisfies DomainFailureFilters;

/** Default order: most messages first. */
export function getDomainFailures(
	id: number,
	params?: DomainFailureFilters & PageParams
): Promise<Page<WithIdentity<DomainFailureRow>>> {
	const query = params && { ...params, outcome: outcomeParam(params.outcome) };
	return api.get(`domains/${id}/failures`, { searchParams: searchParams(query) }).json();
}

/** CSV exports (`GET /api/export/{dataset}.csv`); every one covers the whole scope, no row limit. */
export type ExportDataset = 'domains' | 'senders' | 'auth-issues' | 'dns-findings';

/** Scope of an export. Each dataset reads only the parameters it supports. */
export type ExportScope = ScopeParams & Omit<DomainFailureFilters, 'since'>;

/** The parameters each dataset takes; scope fields outside it are ignored by the export. */
export const EXPORT_PARAMS: Record<ExportDataset, readonly (keyof ExportScope)[]> = {
	domains: ['organization_id', 'since'],
	senders: ['organization_id', 'domain_id', 'since'],
	'auth-issues': ['organization_id', 'domain_id', 'since', 'outcome', 'source_ip'],
	'dns-findings': ['organization_id', 'domain_id']
};

/** Download URL of a CSV export for a scope. */
export function exportCsvUrl(dataset: ExportDataset, scope?: ExportScope): string {
	const picked: Record<string, string | number | null | undefined> = {};
	for (const key of EXPORT_PARAMS[dataset]) {
		picked[key] = key === 'outcome' ? outcomeParam(scope?.outcome) : scope?.[key];
	}
	const qs = new URLSearchParams(searchParams(picked)).toString();
	return `/api/export/${dataset}.csv${qs ? `?${qs}` : ''}`;
}

export function getSystemHealth(): Promise<SystemHealthResponse> {
	return api.get('system/health').json();
}

/**
 * Every setting by group (the password only as `mailbox.has_password`). Read it through the
 * shared `settings` state (`$lib/state/settings.svelte`), which loads it once; each save below
 * answers the whole of it again, for `settings.save()`.
 */
export function getSettings(): Promise<SettingsResponse> {
	return api.get('settings').json();
}

/** The whole mailbox; an absent or blank password keeps the stored one (for the same mailbox only). */
export function saveMailbox(body: SaveMailboxRequest): Promise<SettingsResponse> {
	return api.put('settings/mailbox', { json: body }).json();
}

/** The report address and mode; an absent field keeps the stored value, an empty address clears it. */
export function saveReporting(body: SaveReportingRequest): Promise<SettingsResponse> {
	return api.put('settings/reporting', { json: body }).json();
}

/** The arrival (freshness) warning, stored clamped to its range. */
export function saveMonitoring(body: MonitoringSettings): Promise<SettingsResponse> {
	return api.put('settings/monitoring', { json: body }).json();
}

/** The months reports are kept, stored clamped to their range. */
export function saveRetention(body: RetentionSettings): Promise<SettingsResponse> {
	return api.put('settings/retention', { json: body }).json();
}

/** The resolvers; none goes back to the built-in ones. */
export function saveDns(nameservers: string[]): Promise<SettingsResponse> {
	return api.put('settings/dns', { json: { nameservers } satisfies DnsSettings }).json();
}

/**
 * The SMTP account mail is sent through. An enabled group must be ready to send; an absent
 * or blank password keeps the stored one (for the same server and username only).
 */
export function saveSending(body: SaveSendingRequest): Promise<SettingsResponse> {
	return api.put('settings/sending', { json: body }).json();
}

/**
 * Send the fixed test message to `to` (default: the sender) with the account on screen
 * (`sending`; an absent password is the stored one, for its own account only). A failure is an
 * error answer whose message says which step failed (`apiErrorMessage`).
 */
export function sendTestMail(body: SendTestRequest): Promise<SendTestResponse> {
	// Each step (resolve, connect, TLS, login, hand-over) may take up to 20 seconds.
	return api.post('settings/sending/test', { json: body, timeout: 75_000 }).json();
}

/** Alert emails, the digest and the base URL their links use; the server checks the recipients. */
export function saveAlerts(body: AlertSettings): Promise<SettingsResponse> {
	return api.put('settings/alerts', { json: body }).json();
}

/**
 * Send made-up changes to the recipients on screen (`alerts`) with the saved sending account. A
 * failure is an error answer whose message says which step failed (`apiErrorMessage`), like the
 * sending test.
 */
export function sendTestAlert(body: SendTestAlertRequest): Promise<SendTestResponse> {
	// One message per recipient, each up to a minute on a slow server.
	return api.post('settings/alerts/test', { json: body, timeout: 190_000 }).json();
}

export function saveClientReports(body: ClientReportSettings): Promise<SettingsResponse> {
	return api.put('settings/client-reports', { json: body }).json();
}

/** One organization's client report as it would go out now (sends nothing); 404 for an unknown one. */
export function getClientReportEmail(organizationId: number): Promise<ClientReportEmail> {
	return api.get('settings/client-reports/email', { searchParams: { organization_id: organizationId } }).json();
}

/**
 * Render an organization's client report now and send it to `to` (default: the sender address)
 * with the saved sending account and the Reply-To on screen (`reply_to`). A failure is an error
 * answer like the sending test's.
 */
export function sendClientReportEmail(body: SendClientReportEmailRequest): Promise<SendTestResponse> {
	return api.post('settings/client-reports/email/send', { json: body, timeout: 70_000 }).json();
}

/**
 * The operator key. Without `operator_key` the stored key is kept; an empty one removes it. 422
 * when the key is not valid.
 */
export function saveLicense(body: SaveLicenseRequest): Promise<SettingsResponse> {
	return api.put('settings/license', { json: body }).json();
}

/** The operator name (free for everyone); the logo is kept. */
export function saveBranding(body: BrandingSettings): Promise<SettingsResponse> {
	return api.put('settings/branding', { json: body }).json();
}

/**
 * How the emails look with a name and logo that may not be saved yet: their top and their last
 * line, as the emails render them. Stores nothing; 422 when the name could not be saved.
 */
export function previewBranding(body: BrandingPreviewRequest): Promise<EmailPreview> {
	return api.post('settings/branding/preview', { json: body }).json();
}

/**
 * Replace the logo with `image`, sent as the raw body with its own `image/*` type (the server
 * reads the format from the bytes). 422 with the reason when it is not a PNG, JPEG or WebP image
 * or too large.
 */
export function uploadLogo(image: Blob): Promise<SettingsResponse> {
	return api.post('settings/branding/logo', { body: image, headers: { 'content-type': image.type } }).json();
}

/** Remove the logo. */
export function deleteLogo(): Promise<SettingsResponse> {
	return api.delete('settings/branding/logo').json();
}

/** How the setup ended; `null` opens it again (the dashboard opens /setup). */
export function saveOnboarding(state: OnboardingState | null): Promise<SettingsResponse> {
	return api.put('settings/onboarding', { json: { state } satisfies OnboardingSettings }).json();
}

/**
 * Log in, list folders and count the messages in `folder` (default INBOX). Omitted
 * fields fall back to the stored settings; the stored password is only reused for
 * an unchanged host, port and username.
 */
export function connectImap(body: ConnectImapRequest): Promise<ConnectImapResponse> {
	return api.post('settings/imap/connect', { json: body, timeout: 45_000 }).json();
}

export function syncImap(): Promise<SyncImapResponse> {
	// Every POST is JSON (the server refuses others, against cross-site requests).
	return api.post('settings/imap/sync', { json: {} }).json();
}

/** Read the whole mailbox again and add the reports that are missing. */
export function reimportMailbox(): Promise<SyncImapResponse> {
	return api.post('settings/imap/reimport', { json: {} }).json();
}

// The lists edited as text, one protocol each (`ListText`): GET the list, POST `…/preview` a draft
// (what saving it would change and its line errors; writes nothing), PUT it to save (the same
// summary; a draft with a line error is refused whole, with `errors`, and nothing is written).

function getList(path: string): Promise<ListText> {
	return api.get(path).json();
}

function previewList<S>(path: string, text: string): Promise<S> {
	return api.post(`${path}/preview`, { json: { text } satisfies ListText }).json();
}

function saveList<S>(path: string, text: string): Promise<S> {
	return api.put(path, { json: { text } satisfies ListText }).json();
}

/** The organization ↔ domain mapping, one `organization, domain[, status]` per line. */
export const getDomainMapping = () => getList('settings/domain-mapping');
export const previewDomainMapping = (text: string) => previewList<MappingSummary>('settings/domain-mapping', text);
/** Applied in one transaction; domains left out stay as they are. */
export const saveDomainMapping = (text: string) => saveList<MappingSummary>('settings/domain-mapping', text);

/** The sender name overrides, `target, name` per line (domain, IP or range). */
export const getSenderNames = () => getList('settings/sender-names');
export const previewSenderNames = (text: string) => previewList<SenderNamesSummary>('settings/sender-names', text);
export const saveSenderNames = (text: string) => saveList<SenderNamesSummary>('settings/sender-names', text);

/** The extra DKIM selectors, one per line (stored lowercase, sorted, without repeats). */
export const getDkimSelectors = () => getList('settings/dkim-selectors');
export const previewDkimSelectors = (text: string) =>
	previewList<DkimSelectorsSummary>('settings/dkim-selectors', text);
export const saveDkimSelectors = (text: string) => saveList<DkimSelectorsSummary>('settings/dkim-selectors', text);

/** The client report recipients, `organization,email` per line; the full truth, saving replaces all. */
export const getClientReportRecipients = () => getList('settings/client-reports/recipients');
export const previewClientReportRecipients = (text: string) =>
	previewList<RecipientsSummary>('settings/client-reports/recipients', text);
export const saveClientReportRecipients = (text: string) =>
	saveList<RecipientsSummary>('settings/client-reports/recipients', text);
