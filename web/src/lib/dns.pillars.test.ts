// The scorecard's five pillars from a live check and an overview: every card's verdict,
// rows, points and findings come from here (`pillarModels`).
import { describe, expect, it } from 'vitest';
import type { DnsCheckResponse } from '$lib/types/DnsCheckResponse';
import type { DomainOverview } from '$lib/types/DomainOverview';
import { CHECK_FAILED, pillarModels } from './dns';

/** Enough of a domain overview for the report-data pillar. */
function overview(extra: Record<string, unknown> = {}): DomainOverview {
	return {
		id: 1,
		domain: 'example.com',
		parked: false,
		total_messages: 1000,
		compliant_messages: 900,
		delivered_failures: 20,
		forwarded_failures: 50,
		no_aligned_dkim_messages: 30,
		reporter_count: 4,
		health: { state: 'warning', status: 'warn' },
		delivered_compliance: { pct: 97.8, status: 'warn' },
		assessment: {
			dkim_coverage: { state: 'partial', pct: 96.7, status: 'warn' },
			parked_passing_mail: 'neutral',
			reports_stopped_days: null,
			issues: [
				{
					kind: 'delivered_failures',
					severity: 'high',
					status: 'warn',
					label: '20 delivered failures',
					finding: 'Delivered compliance is 97.8%: 20 messages failed DMARC and were still delivered.'
				}
			]
		},
		...extra
	} as unknown as DomainOverview;
}

/** A complete check with one SPF warning and a TLS-RPT record. */
function check(extra: Record<string, unknown> = {}): DnsCheckResponse {
	return {
		status: 'fresh',
		graded: true,
		grade: 'B',
		score: 95,
		grade_status: 'warn',
		worst_check: 'spf',
		findings: [
			{ check: 'spf', severity: 'warning', code: 'spf_softfail', message: '~all', suggestion: '-all', points: 5 }
		],
		dmarc: {
			records: ['v=DMARC1; p=reject; sp=none; rua=mailto:a@example.net'],
			parsed: { p: 'reject', sp: 'none', pct: 100, rua: [{ raw: 'mailto:a@example.net' }] },
			enforcement: { state: 'reject', status: 'pass' },
			subdomain_enforcement: { state: 'monitor', status: 'fail' },
			pct_below_100: false,
			pct_status: 'neutral',
			rua_authorization: { state: 'authorized', status: 'pass' },
			lookup: 'ok',
			record_status: 'pass'
		},
		spf: {
			records: ['v=spf1 include:_spf.example.net ~all'],
			lookup: 'ok',
			lookup_count: 3,
			lookup_limit: 10,
			void_lookup_count: 0,
			void_limit: 2,
			exceeds_lookup_limit: false,
			all_qualifier: '~',
			all_status: 'warn',
			parsed: { redirect: null }
		},
		dkim: { summary: { state: 'published', status: 'pass' }, observed_selectors: ['s1', 's2'] },
		mta_sts: {
			txt_lookup: 'no_records',
			txt_record: null,
			fetch: { kind: 'not_attempted' },
			fetch_status: 'neutral',
			policy: null,
			mode_status: null
		},
		tls_rpt: { records: ['v=TLSRPTv1; rua=mailto:tls@example.com'], lookup: 'ok' },
		bimi: { records: [], lookup: 'no_records', parsed: null },
		...extra
	} as unknown as DnsCheckResponse;
}

describe('pillarModels', () => {
	it('reads every DNS card as "Check failed" with no points while there is no answer', () => {
		const p = pillarModels(null, overview());
		for (const id of ['dmarc', 'spf', 'dkim', 'transport'] as const) {
			expect(p[id].verdict).toEqual(CHECK_FAILED);
			expect(p[id].lostPts).toBeUndefined();
			expect(p[id].findingsVerdict).toBeNull();
		}
		expect(p.dmarc.rows).toEqual([]);
		expect(p.telemetry.lostPts).toBeNull();
	});

	it('words a complete check: verdicts, rows, points and findings per pillar', () => {
		const p = pillarModels(check(), overview());
		expect(p.dmarc.verdict).toEqual({ label: 'p=reject', tone: 'pass', mono: true });
		expect(p.dmarc.rows).toEqual([
			{ label: 'Report destinations', value: 'Authorized', tone: 'pass' },
			{ label: 'Subdomain policy', value: 'sp=none', tone: 'fail', mono: true }
		]);
		expect(p.dmarc.lostPts).toBe(0);
		expect(p.dmarc.findingsVerdict).toEqual({ label: 'No issues', tone: 'pass' });

		expect(p.spf.verdict).toEqual({ label: '~all', tone: 'warn', mono: true });
		expect(p.spf.rows.map((r) => r.value)).toEqual(['3 of 10', '0 of 2']);
		expect(p.spf.lostPts).toBe(5);
		expect(p.spf.findingsVerdict).toEqual({ label: '1 warning finding', tone: 'warn' });
		expect(p.spf.findings).toHaveLength(1);

		expect(p.dkim.verdict).toEqual({ label: 'Published', tone: 'pass' });
		expect(p.dkim.rows).toEqual([{ label: 'Signing selectors', value: 's1, s2', mono: true }]);

		expect(p.transport.verdict).toEqual({ label: 'No record', tone: 'neutral' });
		expect(p.transport.rows).toEqual([
			{ label: 'TLS-RPT', value: 'Published', tone: 'pass' },
			{ label: 'BIMI', value: 'No record' }
		]);
	});

	it('words the report data from the overview and its assessment', () => {
		const p = pillarModels(check(), overview());
		expect(p.telemetry.verdictLabel).toBe('Delivered compliance');
		expect(p.telemetry.verdict).toEqual({ label: '97.8% (20 delivered failures)', tone: 'warn' });
		expect(p.telemetry.rows).toEqual([
			{ label: 'DKIM signed share', value: '96.7% (30 unsigned)', tone: 'warn' },
			{ label: 'Forwarded', value: '5.0% (50)' },
			{ label: 'Reporters', value: '4' }
		]);
		expect(p.telemetry.findings.map((f) => [f.code, f.severity, f.points])).toEqual([
			['delivered_failures', 'warning', 0],
			['forwarded', 'info', 0]
		]);
		// The server's words, as sent.
		expect(p.telemetry.findings[0].message).toBe(
			'Delivered compliance is 97.8%: 20 messages failed DMARC and were still delivered.'
		);
		expect(p.telemetry.findingsVerdict).toBeNull();
	});

	it('judges a parked domain in reverse: passing mail is the finding, DKIM is not needed', () => {
		const p = pillarModels(
			check(),
			overview({
				parked: true,
				health: { state: 'protected', status: 'pass' },
				delivered_compliance: { pct: null, status: 'neutral' },
				assessment: {
					dkim_coverage: { state: 'none', pct: null, status: 'neutral' },
					parked_passing_mail: 'fail',
					reports_stopped_days: null,
					issues: [
						{
							kind: 'parked_passing_mail',
							severity: 'critical',
							status: 'fail',
							label: '900 passing messages',
							finding: '900 messages passed DMARC for a parked domain: something is authorized to send as it.'
						},
						{ kind: 'parked_not_rejecting', severity: 'critical', status: 'fail', label: 'p=none', finding: null }
					]
				}
			})
		);
		expect(p.dkim.verdict).toEqual({ label: 'Not needed (parked)', tone: 'neutral' });
		expect(p.telemetry.verdictLabel).toBe('Passing mail');
		expect(p.telemetry.verdict).toEqual({ label: '900', tone: 'fail' });
		expect(p.telemetry.findings[0]).toMatchObject({ code: 'parked_passing_mail', severity: 'critical' });
		// An issue without a finding is a DNS card's, not the report data's.
		expect(p.telemetry.findings.map((f) => f.code)).not.toContain('parked_not_rejecting');
	});

	it('reads "No data" when nothing was delivered, and "Check failed" everywhere for a failed check', () => {
		const quiet = overview({
			health: { state: 'no_data', status: 'neutral' },
			delivered_compliance: { pct: null, status: 'neutral' },
			assessment: {
				dkim_coverage: { state: 'none', pct: null, status: 'neutral' },
				parked_passing_mail: 'neutral',
				reports_stopped_days: null,
				issues: []
			}
		});
		expect(pillarModels(check(), quiet).telemetry.verdict).toEqual({ label: 'No data', tone: 'neutral' });

		const failed = pillarModels(check({ status: 'failed' }), overview());
		expect(failed.dmarc.verdict).toEqual(CHECK_FAILED);
		expect(failed.spf.verdict).toEqual(CHECK_FAILED);
		expect(failed.transport.rows).toEqual([]);
		expect(failed.spf.lostPts).toBeUndefined();
		expect(failed.spf.findingsVerdict).toBeNull();
	});
});
