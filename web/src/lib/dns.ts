import type { BimiCheckResult } from '$lib/types/BimiCheckResult';
import type { CheckKind } from '$lib/types/CheckKind';
import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
import type { DomainOverview } from '$lib/types/DomainOverview';
import type { IssueSeverity } from '$lib/types/IssueSeverity';
import type { DkimSelectorResult } from '$lib/types/DkimSelectorResult';
import type { DmarcCheckResult } from '$lib/types/DmarcCheckResult';
import type { Finding } from '$lib/types/Finding';
import type { LookupStatus } from '$lib/types/LookupStatus';
import type { Severity } from '$lib/types/Severity';
import type { MtaStsCheckResult } from '$lib/types/MtaStsCheckResult';
import type { PolicyFetch } from '$lib/types/PolicyFetch';
import type { SpfCheckResult } from '$lib/types/SpfCheckResult';
import type { IncludeNode } from '$lib/types/IncludeNode';
import type { AuthState } from './types/AuthState';
import type { AuthVerdict } from './types/AuthVerdict';
import type { IncludeError } from './types/IncludeError';
import type { SelectorState } from './types/SelectorState';
import type { SelectorVerdict } from './types/SelectorVerdict';
import { formatInt, formatPct, percent, plural } from './format';
import { enforcementLabel, formatRate, HEALTH_LABEL } from './health';
import { SEVERITY_TONE, TONE_RANK, type StatusTone } from './tokens';

/** A short statement with its tone: a card's or row's verdict. `mono` when `label` is literal record text. */
export type Verdict = { label: string; tone: StatusTone; mono?: boolean };

/** The worst of `verdicts` (falsy entries skipped); among equals the first wins, so list the primary first. */
export function worstVerdict(...verdicts: (Verdict | null | undefined | false)[]): Verdict {
	let worst: Verdict | null = null;
	for (const v of verdicts) {
		if (v && (!worst || TONE_RANK[v.tone] > TONE_RANK[worst.tone])) worst = v;
	}
	return worst ?? { label: '—', tone: 'neutral' };
}

/**
 * Each severity's place, worst first: the order of the server's `Severity` (`Severity::ALL`,
 * the order `GET /api/meta` lists `severity_points` in), kept complete by the compiler. A
 * presentation order (findings are listed worst first, and a set of them is named by its worst),
 * never a cutoff: how severe a finding is, is the server's.
 */
export const SEVERITY_RANK = {
	critical: 0,
	warning: 1,
	info: 2,
	notice: 3,
	ok: 4
} as const satisfies Record<Severity, number>;

/** `findings` worst first (stable among equals). */
export function findingsWorstFirst<T extends Pick<Finding, 'severity'>>(findings: readonly T[]): T[] {
	return [...findings].sort((a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity]);
}

/**
 * One verdict for a set of findings: the worst severity, toned by `SEVERITY_TONE` (`$lib/tokens`).
 * No findings (or only `ok`) reads "No issues".
 */
export function findingsVerdict(findings: readonly Pick<Finding, 'severity'>[]): Verdict {
	const worst = findingsWorstFirst(findings)[0]?.severity;
	if (!worst || worst === 'ok') return { label: 'No issues', tone: 'pass' };
	const n = findings.filter((f) => f.severity === worst).length;
	// "1 critical finding": says what kind and how many, not just a severity word.
	return {
		label: plural(n, `${SEVERITY_LABEL[worst].toLowerCase()} finding`),
		tone: SEVERITY_TONE[worst]
	};
}

/** A DNS finding severity, as a word. */
export const SEVERITY_LABEL = {
	critical: 'Critical',
	warning: 'Warning',
	info: 'Info',
	notice: 'Notice',
	ok: 'OK'
} as const satisfies Record<Severity, string>;

/** Score points as shown everywhere: "−28", or "0" when nothing was charged. */
export function formatPoints(points: number): string {
	return points > 0 ? `\u2212${points}` : '0';
}

/**
 * What a findings list shows: a DNS finding, or a report finding (which carries no points).
 */
export type ListedFinding = Pick<Finding, 'severity' | 'code' | 'message' | 'suggestion' | 'points'>;

/** Points `findings` cost in total: the server's per-finding charge, summed. */
export function dnsFindingsDeduction(findings: readonly Pick<Finding, 'points'>[]): number {
	return findings.reduce((sum, f) => sum + f.points, 0);
}

/**
 * Whether a lookup actually produced an answer we can reason about.
 *
 * `nxdomain` and `no_records` count as measured: the name genuinely has no
 * record, and anything derived from that is a real result. `timeout` and
 * `servfail` are not measurements at all, so a derived count is zero only
 * because the walk never ran. Showing those zeros as findings would state a
 * fact about the domain that we never established.
 */
export function dnsLookupMeasured(status: LookupStatus): boolean {
	return LOOKUP_MEASURED[status];
}

const LOOKUP_MEASURED: Record<LookupStatus, boolean> = {
	ok: true,
	nxdomain: true,
	no_records: true,
	timeout: false,
	servfail: false
};

/** Own lookups + all descendants. */
export function spfSubtreeLookups(node: IncludeNode): number {
	let total = node.lookups;
	for (const child of node.children) {
		total += spfSubtreeLookups(child);
	}
	return total;
}

/** Plain-language explanation of why an SPF include branch has no complete record. */
export const INCLUDE_ERROR_EXPLANATION: Record<IncludeError['kind'], (error: IncludeError) => string> = {
	void: () =>
		'This include did not resolve to an SPF record (NXDOMAIN or empty). RFC 7208 counts that as a void lookup.',
	no_spf_record: () =>
		'A TXT answer existed but none of the strings was a v=spf1 record. RFC 7208 counts that as a void lookup.',
	timeout: () => 'The DNS query for this include timed out. The walk could not read a record here.',
	servfail: (e) =>
		`The nameserver returned SERVFAIL for this include (${e.kind === 'servfail' ? e.detail : ''}). The walk could not read a record here.`,
	not_resolved: () => 'This include was never answered: the walk timed out or stopped before reaching it.',
	loop: () => 'This include path loops back to a domain already being evaluated.',
	depth_exceeded: () => 'The walker stopped expanding this branch (depth or walk cap).',
	walk_capped: () => 'The walker stopped expanding this branch (depth or walk cap).'
};

/** {@link INCLUDE_ERROR_EXPLANATION} for a node's error; `null` without one. */
export function spfErrorExplanation(error: IncludeError | undefined): string | null {
	return error ? INCLUDE_ERROR_EXPLANATION[error.kind](error) : null;
}

/** Findings raised by any of `checks` (e.g. `'spf'`, `'mta_sts'`). */
export function findingsFor(findings: readonly Finding[] | undefined, ...checks: string[]) {
	return (findings ?? []).filter((f) => checks.includes(f.check));
}

/**
 * A lookup that timed out or errored: nothing was measured, so it states nothing about the
 * domain (the server rates these `notice`, zero points). Never "Missing" or "DNS error".
 */
export const CHECK_FAILED: Verdict = { label: 'Check failed', tone: 'neutral' };

/**
 * Whether an optional record is published: "Published", "No record" (neutral: nothing
 * requires it) or "Check failed" when the lookup did not complete.
 */
export function recordStatus(
	records: readonly unknown[] | null | undefined,
	lookup: LookupStatus | null | undefined
): Verdict {
	if ((records ?? []).length > 0) return { label: 'Published', tone: 'pass' };
	if (lookup != null && !dnsLookupMeasured(lookup)) return CHECK_FAILED;
	return { label: 'No record', tone: 'neutral' };
}

/** The DMARC record's presence, worded by {@link recordStatus}, coloured by the server's `record_status` (absent fails). */
export function dmarcRecordStatus(check: Pick<DmarcCheckResult, 'records' | 'lookup' | 'record_status'>): Verdict {
	return { ...recordStatus(check.records, check.lookup), tone: check.record_status };
}

/**
 * SPF lookups used, "8 of 10". The server never serves an incomplete include walk as a
 * result (it keeps the last complete one), so the count is always exact.
 */
export function spfLookupsLabel(check: Pick<SpfCheckResult, 'lookup_count' | 'lookup_limit'>): string {
	return `${check.lookup_count} of ${check.lookup_limit}`;
}

/** SPF void lookups, "0 of 2". */
export function spfVoidLookupsLabel(check: Pick<SpfCheckResult, 'void_lookup_count' | 'void_limit'>): string {
	return `${check.void_lookup_count} of ${check.void_limit}`;
}

/** Format the SPF all qualifier for display, e.g. `-` -> `-all`; `null` when there is none. */
function allTerm(qualifier: string | null | undefined): string | null {
	if (qualifier == null || qualifier === '') return null;
	return qualifier.endsWith('all') ? qualifier : `${qualifier}all`;
}

/**
 * What the SPF record does with every sender it does not list (the effective `all`, after
 * following any `redirect=`), coloured by the server's `all_status`.
 */
export function spfAll(
	check: Pick<SpfCheckResult, 'records' | 'lookup' | 'all_qualifier' | 'all_status'> | undefined
): Verdict {
	if (!check || check.records.length === 0) return recordStatus(check?.records, check?.lookup);
	const all = allTerm(check.all_qualifier);
	if (!all) return { label: 'No all mechanism', tone: check.all_status };
	return { label: all, tone: check.all_status, mono: true };
}

/**
 * {@link spfAll} with the `redirect=` that hands the decision to another domain's record,
 * e.g. `redirect=_spf.example.net (-all)`.
 */
export function spfTrailing(
	check: Pick<SpfCheckResult, 'records' | 'lookup' | 'all_qualifier' | 'all_status' | 'parsed'> | undefined
): Verdict {
	const verdict = spfAll(check);
	const redirect = check?.parsed?.redirect;
	if (!redirect || !check?.records.length) return verdict;
	const all = allTerm(check.all_qualifier);
	return { label: all ? `redirect=${redirect} (${all})` : `redirect=${redirect}`, tone: verdict.tone, mono: true };
}

/**
 * MTA-STS: the policy's `mode=`; else, with a record, whether its policy host could be
 * reached (the server's `fetch_status`); else whether the TXT record is published. Our own
 * fetch timeout measured nothing and reads as a failed check.
 */
export function mtaStsStatus(
	m:
		| Pick<MtaStsCheckResult, 'txt_lookup' | 'txt_record' | 'fetch' | 'fetch_status' | 'policy' | 'mode_status'>
		| undefined
): Verdict {
	if (!m) return CHECK_FAILED;
	const mode = m.policy?.mode;
	if (mode) return { label: `mode=${mode}`, tone: m.mode_status ?? 'neutral', mono: true };
	if (m.txt_record) {
		if (m.fetch.kind === 'timeout') return CHECK_FAILED;
		if (m.fetch_status !== 'neutral') return { label: 'Policy unreachable', tone: m.fetch_status };
	}
	return recordStatus(m.txt_record ? [m.txt_record] : [], m.txt_lookup);
}

/** BIMI: an empty `l=` declares that the domain declines BIMI; otherwise whether a record is published. */
export function bimiStatus(b: Pick<BimiCheckResult, 'records' | 'lookup' | 'parsed'> | undefined): Verdict {
	if (!b) return CHECK_FAILED;
	if (b.parsed?.declined) return { label: 'Declined', tone: 'neutral' };
	return recordStatus(b.records, b.lookup);
}

/** The MTA-STS policy fetch outcome as plain text, e.g. `HTTP 404`. Not a verdict. */
export const POLICY_FETCH_LABEL: Record<PolicyFetch['kind'], (f: PolicyFetch) => string> = {
	not_attempted: () => 'not fetched',
	ok: () => 'OK',
	name_not_found: () => 'name not found',
	private_address: (f) => `private address${f.kind === 'private_address' ? ` ${f.address}` : ''}`,
	connect_failed: () => 'no connection',
	tls_failed: () => 'TLS failed',
	redirect: () => 'redirect',
	http_status: (f) => `HTTP ${f.kind === 'http_status' ? f.code : ''}`,
	too_large: () => 'too large',
	timeout: () => 'timeout',
	failed: () => 'failed'
};

/** {@link POLICY_FETCH_LABEL} for one fetch. */
export function policyFetchLabel(f: PolicyFetch): string {
	return POLICY_FETCH_LABEL[f.kind](f);
}

type DkimKey = Pick<DkimSelectorResult, 'key_type' | 'key_bits' | 'key_status'>;

/** Algorithm and size of a published key, "RSA 2048-bit" or "Ed25519", coloured by the server's `key_status`. */
export function dkimKey(key: DkimKey): Verdict {
	const tone = key.key_status;
	if (key.key_type?.toLowerCase() === 'ed25519') return { label: 'Ed25519', tone };
	const label = `${key.key_type?.toUpperCase() ?? 'RSA'} ${key.key_bits ? `${key.key_bits}-bit` : 'key, size unknown'}`;
	return { label, tone };
}

/**
 * Why a DKIM selector was checked. Nothing is guessed: a selector is either
 * seen signing in aggregate reports, or on the user's selector list.
 */
export function dkimSelectorOrigin(rec: Pick<DkimSelectorResult, 'seen_in_reports' | 'configured'>): string {
	if (rec.seen_in_reports && rec.configured) return 'Seen in reports · your list';
	if (rec.seen_in_reports) return 'Seen in reports';
	if (rec.configured) return 'Your selector list';
	return 'Requested';
}

/** What one probed DKIM selector (or the domain's selectors as a whole) says. */
export const SELECTOR_LABEL = {
	published: 'Published',
	not_published: 'No record',
	revoked: 'Revoked',
	weak_key: 'Weak key',
	test_mode: 'Test mode',
	dated_key: 'Dated key',
	check_failed: 'Check failed',
	no_selectors: 'No selectors'
} as const satisfies Record<SelectorState, string>;

/** A selector's (or the selectors') verdict from the server, worded. */
export function selectorVerdict(verdict: SelectorVerdict): Verdict {
	return { label: SELECTOR_LABEL[verdict.state], tone: verdict.status };
}

/** RFC 7489 §7.1 authorization of an external report destination, as a word. */
export const AUTH_LABEL = {
	authorized: 'Authorized',
	not_authorized: 'Not authorized',
	check_failed: 'Check failed'
} as const satisfies Record<AuthState, string>;

/** RFC 7489 §7.1 verdict for one external rua destination (or all of them), worded. */
export function authVerdict(verdict: AuthVerdict): Verdict {
	return { label: AUTH_LABEL[verdict.state], tone: verdict.status };
}

/**
 * One verdict across the record's rua destinations (the server's `rua_authorization`).
 * "Internal" when every address is under the domain, "—" when there is no rua at all.
 */
export function ruaAuthSummary(check: Pick<DmarcCheckResult, 'parsed' | 'rua_authorization'> | undefined): Verdict {
	if (check?.rua_authorization) return authVerdict(check.rua_authorization);
	return check?.parsed?.rua.length ? { label: 'Internal', tone: 'neutral' } : { label: '—', tone: 'neutral' };
}

// ---------------------------------------------------------------------------
// Scorecard pillars
// ---------------------------------------------------------------------------

export type PillarId = 'dmarc' | 'spf' | 'dkim' | 'transport' | 'telemetry';

/** The scorecard's pillars in display order, with the one name each shows. */
export const PILLAR_LABEL: Record<PillarId, string> = {
	dmarc: 'DMARC',
	spf: 'SPF',
	dkim: 'DKIM',
	transport: 'Transport',
	telemetry: 'Report data'
};
export const PILLAR_IDS: PillarId[] = ['dmarc', 'spf', 'dkim', 'transport', 'telemetry'];

/**
 * A pillar card row: plain sans text (`mono` for literal record/tag text). A `tone` colours
 * the value and makes it part of the card's verdict.
 */
export type PillarRow = {
	label: string;
	value: string;
	tone?: StatusTone;
	mono?: boolean;
};

/** Everything one scorecard pillar shows: its card and the findings at the top of its drawer. */
export type PillarModel = {
	/** The pillar's own verdict, the card's first row. */
	verdict: Verdict;
	verdictLabel: string;
	rows: PillarRow[];
	/** What is wrong, listed first in the drawer (report findings carry no points). */
	findings: ListedFinding[];
	/** The card's findings verdict; `null` while no complete check backs one (or for report data). */
	findingsVerdict: Verdict | null;
	/** Score points the pillar costs; `null` for a pillar that is not scored, `undefined` while unknown. */
	lostPts: number | null | undefined;
};

/** The pillar card each DNS check is shown on. */
export const CHECK_PILLAR: Record<CheckKind, PillarId> = {
	dmarc: 'dmarc',
	spf: 'spf',
	dkim: 'dkim',
	mta_sts: 'transport',
	tls_rpt: 'transport',
	bimi: 'transport'
};

/** A report issue as a finding of the same weight (the issue's own severity). */
const ISSUE_FINDING_SEVERITY: Record<IssueSeverity, Severity> = {
	critical: 'critical',
	high: 'warning',
	medium: 'warning'
};

/** The DMARC card's verdict ("Policy"): the enforcement in words, else whether a record is published. */
export function dmarcPolicyVerdict(dns: DnsCheckResponse | null): Verdict {
	const dmarc = dns?.dmarc;
	if (!dns || dns.status === 'failed' || !dmarc) return CHECK_FAILED;
	if (!dmarc.records.length) return dmarcRecordStatus(dmarc);
	return { label: enforcementLabel(dmarc.enforcement, dmarc.parsed?.p), tone: dmarc.enforcement.status, mono: true };
}

/** The SPF card's verdict ("Trailing all"): {@link spfAll} of a complete check. */
export function spfAllVerdict(dns: DnsCheckResponse | null): Verdict {
	if (!dns || dns.status === 'failed' || !dns.spf) return CHECK_FAILED;
	return spfAll(dns.spf);
}

/**
 * The five pillars of a domain's scorecard from its live DNS check and its overview. The
 * score is DNS configuration only: the four DNS deductions add up to 100 - score (the
 * server charges each finding code once). Report data is status, not points. A check that
 * failed outright (`dns.status === 'failed'`, no complete result) reads "Check failed" on
 * every DNS card; `null` (no answer yet) does too, the cards show skeletons meanwhile.
 */
export function pillarModels(dns: DnsCheckResponse | null, overview: DomainOverview): Record<PillarId, PillarModel> {
	const failed = dns?.status === 'failed';
	/** A complete result to rate. */
	const checked = dns !== null && !failed;
	const dnsPillar = (findings: Finding[], verdict: Verdict, verdictLabel: string, rows: PillarRow[]): PillarModel => ({
		verdict,
		verdictLabel,
		rows,
		findings,
		findingsVerdict: checked ? findingsVerdict(findings) : null,
		lostPts: checked ? dnsFindingsDeduction(findings) : undefined
	});

	// DMARC
	const dmarc = dns?.dmarc;
	const parsed = dmarc?.parsed;
	const dmarcVerdict = dmarcPolicyVerdict(dns);
	const dmarcRows: PillarRow[] = [];
	if (dmarc && parsed && !failed) {
		const rua = ruaAuthSummary(dmarc);
		const sp = dmarc.subdomain_enforcement;
		const np = dmarc.nonexistent_enforcement;
		dmarcRows.push({ label: 'Report destinations', value: rua.label, tone: rua.tone });
		dmarcRows.push(
			sp
				? { label: 'Subdomain policy', value: `sp=${parsed.sp}`, tone: sp.status, mono: true }
				: { label: 'Subdomain policy', value: 'Same as p=' }
		);
		if (np) {
			dmarcRows.push({ label: 'Non-existent subdomains', value: `np=${parsed.np}`, tone: np.status, mono: true });
		}
		if (dmarc.pct_below_100 && parsed.pct != null) {
			dmarcRows.push({ label: 'Applied to', value: `pct=${parsed.pct}`, tone: dmarc.pct_status, mono: true });
		}
		if (dmarc.testing_status !== 'neutral') {
			dmarcRows.push({ label: 'Testing', value: 't=y', tone: dmarc.testing_status, mono: true });
		}
	}

	// SPF: the effective `all` is the verdict (redirects are followed to find it; the target
	// itself is in the drawer's record and include tree).
	const spf = dns?.spf;
	const spfRows: PillarRow[] =
		spf && spf.records.length && dnsLookupMeasured(spf.lookup) && !failed
			? [
					{ label: 'DNS lookups', value: spfLookupsLabel(spf), tone: spf.exceeds_lookup_limit ? 'fail' : undefined },
					{ label: 'Void lookups', value: spfVoidLookupsLabel(spf) }
				]
			: [];

	// DKIM: the selectors this domain signs with (from reports); the Settings list applies to all domains.
	const dkim = dns?.dkim;
	const dkimVerdict: Verdict =
		!dkim || failed
			? CHECK_FAILED
			: overview.parked
				? { label: 'Not needed (parked)', tone: 'neutral' }
				: selectorVerdict(dkim.summary);
	const seen = dkim?.observed_selectors ?? [];
	const dkimRows: PillarRow[] = [
		{ label: 'Signing selectors', value: seen.length > 0 ? seen.join(', ') : '—', mono: seen.length > 0 }
	];

	// Transport
	const mtaSts = checked && dns ? mtaStsStatus(dns.mta_sts) : CHECK_FAILED;
	const tls = dns ? recordStatus(dns.tls_rpt.records, dns.tls_rpt.lookup) : CHECK_FAILED;
	const bimi = dns ? bimiStatus(dns.bimi) : CHECK_FAILED;
	const transportRows: PillarRow[] = failed
		? []
		: [
				{ label: 'TLS-RPT', value: tls.label, tone: tls.tone },
				// BIMI is cosmetic: inventory, never a verdict.
				{ label: 'BIMI', value: bimi.label }
			];

	// Telemetry: the overview's figures and the server's verdicts on them for the selected
	// window (`overview.assessment`), the same as every other screen. Its row label says what
	// it measures, so the value is the figure alone.
	const assessment = overview.assessment;
	let reportVerdict: Verdict;
	if (overview.parked) {
		reportVerdict = {
			label: overview.compliant_messages > 0 ? formatInt(overview.compliant_messages) : 'None',
			tone: assessment.parked_passing_mail
		};
	} else if (overview.delivered_compliance.pct == null) {
		// Nothing delivered: "All blocked" when mail was seen, "No data" when none was.
		reportVerdict = { label: HEALTH_LABEL[overview.health.state], tone: overview.health.status };
	} else {
		const rate = overview.delivered_compliance;
		reportVerdict = {
			label: `${formatRate(rate)} (${plural(overview.delivered_failures, 'delivered failure')})`,
			tone: rate.status
		};
	}
	/** Share of the passing mail signed by this domain; not shown for parked domains. */
	const signedShare = overview.parked || assessment.dkim_coverage.pct == null ? null : assessment.dkim_coverage;
	/** The open reports-stopped issue (never for a parked domain). */
	const stopped = assessment.issues.find((i) => i.kind === 'reports_stopped');
	// Rated rows colour the card; forwarded and reporters are information only.
	const reportRows: PillarRow[] = [];
	if (signedShare != null) {
		reportRows.push({
			label: 'DKIM signed share',
			value: `${formatRate(signedShare)} (${formatInt(overview.no_aligned_dkim_messages)} unsigned)`,
			tone: signedShare.status
		});
	}
	if (overview.forwarded_failures > 0) {
		reportRows.push({
			label: 'Forwarded',
			value: `${formatPct(percent(overview.forwarded_failures, overview.total_messages))} (${formatInt(overview.forwarded_failures)})`
		});
	}
	if (stopped && assessment.reports_stopped_days != null) {
		reportRows.push({
			label: 'Last report',
			value: `${assessment.reports_stopped_days} days ago`,
			tone: stopped.status
		});
	}
	reportRows.push({ label: 'Reporters', value: formatInt(overview.reporter_count) });

	// Report data as findings, like the DNS checks but without points: the drawer lists what is
	// below the line first. A finding can be legitimate (a forwarder, spam rightly stopped) or a
	// real problem; the drawer is where that is decided. Which report issues are open, how bad,
	// and their words are the server's (`issue.finding`; DNS and enforcement issues have none,
	// they are the DNS cards' own findings).
	const reportFindings: ListedFinding[] = [];
	const finding = (severity: Severity, code: string, message: string): ListedFinding => ({
		severity,
		code,
		message,
		suggestion: '',
		points: 0
	});
	for (const issue of assessment.issues) {
		if (issue.finding) reportFindings.push(finding(ISSUE_FINDING_SEVERITY[issue.severity], issue.kind, issue.finding));
	}
	if (overview.forwarded_failures > 0) {
		reportFindings.push(
			finding(
				'info',
				'forwarded',
				`${formatInt(overview.forwarded_failures)} messages failed only because a forwarder or mailing list changed them; the receivers trusted the forward.`
			)
		);
	}

	return {
		dmarc: dnsPillar(findingsFor(dns?.findings, 'dmarc'), dmarcVerdict, 'Policy', dmarcRows),
		spf: dnsPillar(findingsFor(dns?.findings, 'spf'), spfAllVerdict(dns), 'Trailing all', spfRows),
		dkim: dnsPillar(findingsFor(dns?.findings, 'dkim'), dkimVerdict, 'Keys', dkimRows),
		transport: dnsPillar(findingsFor(dns?.findings, 'mta_sts', 'tls_rpt', 'bimi'), mtaSts, 'MTA-STS', transportRows),
		telemetry: {
			verdict: reportVerdict,
			verdictLabel: overview.parked ? 'Passing mail' : 'Delivered compliance',
			rows: reportRows,
			findings: reportFindings,
			findingsVerdict: null,
			lostPts: null
		}
	};
}
