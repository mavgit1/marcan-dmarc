// The request functions of the JSON API, one per endpoint. Response and request types are
// generated from the Rust API by ts-rs into `#lib/types` (`cargo test` writes them; never edit
// them by hand) and imported from there, here and at every call site. The client, the query
// serializer and the error classifiers live in `#lib/http.ts`.
import { api, searchParams } from '#lib/http.ts';
import type { DkimSelectorsSummary } from '#lib/types/DkimSelectorsSummary.ts';
import type { ListText } from '#lib/types/ListText.ts';
import type { MappingSummary } from '#lib/types/MappingSummary.ts';
import type { RecipientsSummary } from '#lib/types/RecipientsSummary.ts';
import type { SendClientReportEmailRequest } from '#lib/types/SendClientReportEmailRequest.ts';
import type { SenderNamesSummary } from '#lib/types/SenderNamesSummary.ts';
import type { AlertSettings } from '#lib/types/AlertSettings.ts';
import type { BrandingSettings } from '#lib/types/BrandingSettings.ts';
import type { ClientReportSettings } from '#lib/types/ClientReportSettings.ts';
import type { DnsSettings } from '#lib/types/DnsSettings.ts';
import type { UpdateSettings } from '#lib/types/UpdateSettings.ts';
import type { AlertItem } from '#lib/types/AlertItem.ts';
import type { BimiCheckResult } from '#lib/types/BimiCheckResult.ts';
import type { AccountTestRequest } from '#lib/types/AccountTestRequest.ts';
import type { AccountTestResponse } from '#lib/types/AccountTestResponse.ts';
import type { CertificateDraft } from '#lib/types/CertificateDraft.ts';
import type { GenerateCertificateRequest } from '#lib/types/GenerateCertificateRequest.ts';
import type { ClientReportEmail } from '#lib/types/ClientReportEmail.ts';
import type { BrandingPreviewRequest } from '#lib/types/BrandingPreviewRequest.ts';
import type { CreateDomainBody } from '#lib/types/CreateDomainBody.ts';
import type { CreatedDomain } from '#lib/types/CreatedDomain.ts';
import type { DkimCheckResult } from '#lib/types/DkimCheckResult.ts';
import type { DmarcCheckResult } from '#lib/types/DmarcCheckResult.ts';
import type { DnsCheckResponse } from '#lib/types/DnsCheckResponse.ts';
import type { DomainFailureRow } from '#lib/types/DomainFailureRow.ts';
import type { DomainOverview } from '#lib/types/DomainOverview.ts';
import type { DomainSummary } from '#lib/types/DomainSummary.ts';
import type { EmailPreview } from '#lib/types/EmailPreview.ts';
import type { FailureOutcome } from '#lib/types/FailureOutcome.ts';
import type { GuideResponse } from '#lib/types/GuideResponse.ts';
import type { Items } from '#lib/types/Items.ts';
import type { Meta } from '#lib/types/Meta.ts';
import type { MonitoringSettings } from '#lib/types/MonitoringSettings.ts';
import type { MtaStsCheckResult } from '#lib/types/MtaStsCheckResult.ts';
import type { OnboardingSettings } from '#lib/types/OnboardingSettings.ts';
import type { OnboardingState } from '#lib/types/OnboardingState.ts';
import type { OrganizationNameRequest } from '#lib/types/OrganizationNameRequest.ts';
import type { OrganizationRow } from '#lib/types/OrganizationRow.ts';
import type { OrganizationSummary } from '#lib/types/OrganizationSummary.ts';
import type { Page } from '#lib/types/Page.ts';
import type { PtrLookup } from '#lib/types/PtrLookup.ts';
import type { ReportDetailResponse } from '#lib/types/ReportDetailResponse.ts';
import type { ReportSummary } from '#lib/types/ReportSummary.ts';
import type { RetentionSettings } from '#lib/types/RetentionSettings.ts';
import type { ReportingSetupResponse } from '#lib/types/ReportingSetupResponse.ts';
import type { SaveLicenseRequest } from '#lib/types/SaveLicenseRequest.ts';
import type { ReceivingSettings } from '#lib/types/ReceivingSettings.ts';
import type { SaveAccountRequest } from '#lib/types/SaveAccountRequest.ts';
import type { SendingSettings } from '#lib/types/SendingSettings.ts';
import type { ScopeOptions } from '#lib/types/ScopeOptions.ts';
import type { SendTestAlertRequest } from '#lib/types/SendTestAlertRequest.ts';
import type { SendTestRequest } from '#lib/types/SendTestRequest.ts';
import type { SendTestResponse } from '#lib/types/SendTestResponse.ts';
import type { SenderDetailResponse } from '#lib/types/SenderDetailResponse.ts';
import type { SenderSummary } from '#lib/types/SenderSummary.ts';
import type { SettingsResponse } from '#lib/types/SettingsResponse.ts';
import type { SpfCheckResult } from '#lib/types/SpfCheckResult.ts';
import type { Statistics } from '#lib/types/Statistics.ts';
import type { SyncResponse } from '#lib/types/SyncResponse.ts';
import type { SystemHealthResponse } from '#lib/types/SystemHealthResponse.ts';
import type { UpcomingClientReport } from '#lib/types/UpcomingClientReport.ts';
import type { TimeseriesPoint } from '#lib/types/TimeseriesPoint.ts';
import type { TlsRptCheckResult } from '#lib/types/TlsRptCheckResult.ts';
import type { UpdateDomainBody } from '#lib/types/UpdateDomainBody.ts';
import type { WithIdentity } from '#lib/types/WithIdentity.ts';

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
/** `q` searches the source IP and the sender's name on the server. */
export type SendersParams = ScopeParams & PageParams & { failing?: boolean; q?: string | null };

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

/** What only the guide shows: the monitored domains counted by policy. */
export function getGuide(): Promise<GuideResponse> {
	return api.get('guide').json();
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

/**
 * The scope's domains with an open issue in their attention rank over the whole scope, a page of
 * them (`limit` from `offset`, narrowed by `q` like the domain list) and `total`.
 */
export function getAttentionDomains(
	params?: ScopeParams & { limit?: number; offset?: number; q?: string | null }
): Promise<Page<DomainSummary>> {
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

/** The parameters each dataset takes; the server refuses any other (a 400), so only these are sent. */
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
 * Every setting by group, with the accounts (a password only as `has_password`). Read it through the
 * shared `settings` state (`#lib/state/settings.svelte.ts`), which loads it once; each save below
 * answers the whole of it again, for `settings.save()`.
 */
export function getSettings(): Promise<SettingsResponse> {
	return api.get('settings').json();
}

/**
 * Add an account. The server checks it (a 422 or 409 says what is wrong) and answers every
 * setting, the new account last in `accounts`.
 */
export function createAccount(body: SaveAccountRequest): Promise<SettingsResponse> {
	return api.post('accounts', { json: body }).json();
}

/**
 * Replace account `id`; an absent or blank password keeps the stored one, which is only allowed for
 * the servers and username it was saved for (a 409 otherwise), and no draft certificate keeps the
 * stored certificate.
 */
export function updateAccount(id: number, body: SaveAccountRequest): Promise<SettingsResponse> {
	return api.put(`accounts/${id}`, { json: body }).json();
}

/** Remove account `id` (its reports stay); a 409 while sending goes through it. */
export function deleteAccount(id: number): Promise<SettingsResponse> {
	return api.delete(`accounts/${id}`).json();
}

/**
 * Test the account on screen: the token (Microsoft), the mailbox with its folders (IMAP, or Graph
 * for Microsoft), SMTP (no message sent). Nothing is stored; each step answers its own line.
 */
export function testAccount(body: AccountTestRequest): Promise<AccountTestResponse> {
	// The mailbox step may take 30 seconds, the SMTP login 60.
	return api.post('accounts/test', { json: body, timeout: 100_000 }).json();
}

/**
 * A key pair and certificate for a Microsoft login named `subject`, held by the server until the
 * account is saved with its `draft_id`, which stores them; the private key never leaves it.
 */
export function generateCertificate(body: GenerateCertificateRequest): Promise<CertificateDraft> {
	return api.post('accounts/certificate', { json: body, timeout: 60_000 }).json();
}

/** Where an unsaved form's certificate downloads (`.cer`): to renew without a gap, uploaded before the save. */
export function draftCertificateUrl(draftId: string): string {
	return `/api/accounts/certificate/${encodeURIComponent(draftId)}`;
}

/** Where a saved account's certificate downloads (`.cer`). */
export function accountCertificateUrl(id: number): string {
	return `/api/accounts/${id}/certificate.cer`;
}

/** How often every receiving account is read, stored clamped to its range. */
export function saveReceiving(body: ReceivingSettings): Promise<SettingsResponse> {
	return api.put('settings/receiving', { json: body }).json();
}

/** The arrival (freshness) warning, stored clamped to its range. */
export function saveMonitoring(body: MonitoringSettings): Promise<SettingsResponse> {
	return api.put('settings/monitoring', { json: body }).json();
}

/** The months reports are kept, stored clamped to their range. */
export function saveRetention(body: RetentionSettings): Promise<SettingsResponse> {
	return api.put('settings/retention', { json: body }).json();
}

/** The daily version check (on by default); switched off, its last answer is forgotten. */
export function saveUpdates(body: UpdateSettings): Promise<SettingsResponse> {
	return api.put('settings/updates', { json: body }).json();
}

/** The resolvers; none goes back to the built-in ones. */
export function saveDns(nameservers: string[]): Promise<SettingsResponse> {
	return api.put('settings/dns', { json: { nameservers } satisfies DnsSettings }).json();
}

/** The sending group: on or off, the account it goes through and its addresses; an enabled group must be ready to send. */
export function saveSending(body: SendingSettings): Promise<SettingsResponse> {
	return api.put('settings/sending', { json: body }).json();
}

/**
 * Send the fixed test message to the test recipient (default: the sender) with the group on
 * screen (`sending`, its account's stored login), else the stored group. A failure is an
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

/**
 * URL of the stored logo's image (404 without one), for an `<img>`; `revision` makes a new URL
 * after a save, so a page that showed the old logo asks again.
 */
export function logoUrl(revision: number): string {
	return `/api/settings/branding/logo?r=${revision}`;
}

/** Remove the logo. */
export function deleteLogo(): Promise<SettingsResponse> {
	return api.delete('settings/branding/logo').json();
}

/** How the setup ended; `null` opens it again (the dashboard opens /setup). */
export function saveOnboarding(state: OnboardingState | null): Promise<SettingsResponse> {
	return api.put('settings/onboarding', { json: { state } satisfies OnboardingSettings }).json();
}

export function syncAll(): Promise<SyncResponse> {
	// Every POST is JSON (the server refuses others, against cross-site requests).
	return api.post('settings/receiving/sync', { json: {} }).json();
}

/** Read the whole mailbox again and add the reports that are missing. */
export function reimportMailbox(): Promise<SyncResponse> {
	return api.post('settings/receiving/reimport', { json: {} }).json();
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

/**
 * The client report recipients, `organization,email` or `organization,none` per line; a save changes
 * only the organizations it names.
 */
export const getClientReportRecipients = () => getList('settings/client-reports/recipients');
export const previewClientReportRecipients = (text: string) =>
	previewList<RecipientsSummary>('settings/client-reports/recipients', text);
export const saveClientReportRecipients = (text: string) =>
	saveList<RecipientsSummary>('settings/client-reports/recipients', text);
