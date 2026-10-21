<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import { Button } from '$lib/components/ui/button/index.js';
	import { DOCS_URL } from '$lib/nav';
	import type { Severity } from '$lib/types/Severity';
	import DnsRawRecord from '$lib/components/dns-raw-record.svelte';
	import { formatPoints } from '$lib/dns';
	import SetupStepper from '$lib/components/setup/setup-stepper.svelte';
	import { meta as rules } from '$lib/state/meta.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import { focusUrlHash, onFocusAnchor } from '$lib/state/anchor-focus.svelte';

	/**
	 * Tools → Setup: the setup's own steps over what is set up now, read-only (the same
	 * stepper as /setup, saving nothing), then a short DMARC, SPF and DKIM reference.
	 */

	/** Reference sections; ids are linked from other pages (`/guide#dmarc`, `#spf`, `#rollout`, …). */
	const REFERENCE_IDS = ['dmarc', 'authorization', 'spf', 'dkim', 'rollout', 'score', 'optional'] as const;
	type RefId = (typeof REFERENCE_IDS)[number];
	let open = $state<Record<RefId, boolean>>({
		dmarc: false,
		authorization: false,
		spf: false,
		dkim: false,
		rollout: false,
		score: false,
		optional: false
	});

	/**
	 * A reference section opens and scrolls into view when the URL's hash names it (links from
	 * other pages land here), the way Settings shows its sections.
	 */
	for (const id of REFERENCE_IDS)
		onFocusAnchor(
			() => id,
			() => document.getElementById(id),
			() => (open[id] = true)
		);
	$effect(() => focusUrlHash(page.url.hash));

	const dmarcTags = [
		{ tag: 'v=DMARC1', text: 'Marks the record; must come first.' },
		{ tag: 'p=', text: 'Policy for failing mail: none (monitor), quarantine (spam), reject (refuse).' },
		{
			tag: 'rua=',
			text: 'Where aggregate reports go. Several addresses are allowed, comma-separated: mailto:a@x.com,mailto:b@y.net.'
		},
		{ tag: 'sp=', text: 'Policy for subdomains; inherits p= when absent.' },
		{ tag: 'np=', text: 'Policy for subdomains that do not exist (RFC 9989); inherits sp= when absent.' },
		{
			tag: 'pct=',
			text: 'Share of failing mail the policy applies to (default 100). Receivers following RFC 9989 ignore it.'
		},
		{ tag: 't=y', text: 'Testing (RFC 9989): receivers following it do not apply the policy, like pct=0.' },
		{
			tag: 'adkim= / aspf=',
			text: 'Alignment: r (relaxed, same organizational domain, default) or s (strict, exact match).'
		}
	];

	/** The rules' numbers, from the server (never copied here); the text reads "…" without them. */
	const meta = $derived(rules.current);
	$effect(() => {
		rules.load().catch(() => {});
	});
	/** "1024", or "…" until the rules have loaded. */
	const num = (n: number | undefined) => (n == null ? '…' : String(n));

	/** "−28", "0", or "…" until the rules have loaded. */
	function pts(severity: Severity): string {
		const n = meta?.severity_points.find((p) => p.severity === severity)?.points;
		return n == null ? '…' : formatPoints(n);
	}
	const gradeBands = $derived(meta ? meta.grade_bands.map((b) => `${b.grade} ${b.min_score}+`).join(', ') : '…');
	const scoreRows = $derived([
		{ severity: 'Critical', points: pts('critical'), example: 'No DMARC, p=none, SPF over 10 lookups, +all' },
		{
			severity: 'Warning',
			points: pts('warning'),
			example: `~all, pct below 100, ${num(meta?.rsa_min_bits)}-bit DKIM key`
		},
		{ severity: 'Info', points: pts('info'), example: 'MTA-STS or TLS-RPT not published' },
		{ severity: 'Notice', points: pts('notice'), example: 'BIMI, failed DNS checks, unknown tags' }
	]);
</script>

{#snippet refSection(id: RefId, title: string, body: import('svelte').Snippet)}
	<details {id} bind:open={open[id]} class="group scroll-mt-below-header border-b last:border-b-0">
		<summary
			class="hover:bg-muted/40 flex cursor-pointer list-none items-center gap-2 px-4 py-3 font-medium [&::-webkit-details-marker]:hidden"
		>
			<ChevronRightIcon class="text-muted-foreground size-4 transition-transform group-open:rotate-90" />
			{title}
		</summary>
		<div class="text-muted-foreground space-y-2 px-4 pb-4 pl-10">{@render body()}</div>
	</details>
{/snippet}

<div class="mx-auto max-w-3xl space-y-8 text-sm leading-relaxed">
	<SetupStepper readonly>
		{#snippet header()}
			<div class="flex flex-wrap gap-2">
				<Button href={resolve('/setup')} variant="outline" size="sm">Open the setup</Button>
				<Button href={DOCS_URL} target="_blank" rel="noopener" variant="outline" size="sm">
					Documentation
					<ExternalLinkIcon />
				</Button>
			</div>
		{/snippet}
	</SetupStepper>

	<section class="space-y-3">
		<h2 class="text-sm font-medium">How it works</h2>
		<div class="bg-card rounded-lg border">
			{#snippet dmarc()}
				<p>
					One TXT record at <InlineCode>{'_dmarc.<domain>'}</InlineCode>, a list of <InlineCode
						>{'tag=value'}</InlineCode
					> pairs separated by semicolons.
				</p>
				<dl class="space-y-1">
					{#each dmarcTags as t (t.tag)}
						<div class="grid gap-x-3 sm:grid-cols-[8rem_minmax(0,1fr)]">
							<dt><InlineCode>{t.tag}</InlineCode></dt>
							<dd>{t.text}</dd>
						</div>
					{/each}
				</dl>
				<p>Publish exactly one record; with two or more, receivers ignore DMARC for the domain.</p>
			{/snippet}
			{@render refSection('dmarc', 'DMARC record tags', dmarc)}

			{#snippet authorization()}
				<p>
					RFC 7489 §7.1: when a domain's <InlineCode>{'rua='}</InlineCode> points to a mailbox on another domain, receivers
					first look up <InlineCode>{'<domain>._report._dmarc.<mailbox-domain>'}</InlineCode> and only send reports if it
					holds <InlineCode>{'v=DMARC1'}</InlineCode>. Otherwise reports are silently dropped.
				</p>
				<p>
					This stops anyone from flooding a third party with reports. A wildcard
					<InlineCode>{'*._report._dmarc.<mailbox-domain>'}</InlineCode> authorizes every domain at once.
				</p>
			{/snippet}
			{@render refSection('authorization', 'Why the mailbox domain must authorize reports', authorization)}

			{#snippet spf()}
				<p>
					One TXT record at <InlineCode>{'<domain>'}</InlineCode> listing every sender (<InlineCode>{'ip4:'}</InlineCode
					>, <InlineCode>{'ip6:'}</InlineCode>, <InlineCode>{'include:'}</InlineCode>).
				</p>
				<DnsRawRecord
					value="v=spf1 include:_spf.google.com include:spf.protection.outlook.com -all"
					label="Example value"
				/>
				<p>
					End with <InlineCode>{'-all'}</InlineCode>; <InlineCode>{'~all'}</InlineCode> is a warning, <InlineCode
						>{'+all'}</InlineCode
					> critical. Max 10 DNS lookups (<InlineCode>{'include'}</InlineCode>, <InlineCode>{'a'}</InlineCode>, <InlineCode
						>{'mx'}</InlineCode
					>, <InlineCode>{'exists'}</InlineCode>,
					<InlineCode>{'redirect'}</InlineCode>, nested too); past 10 SPF fails for every message.
				</p>
				<p>
					<InlineCode>{'redirect=other.domain'}</InlineCode> uses that domain's record including its <InlineCode
						>{'all'}</InlineCode
					>; don't combine the two.
				</p>
			{/snippet}
			{@render refSection('spf', 'SPF', spf)}

			{#snippet dkim()}
				<p>
					Each sending service signs with its own key, published at
					<InlineCode>{'<selector>._domainkey.<domain>'}</InlineCode> (TXT or CNAME); the provider gives you both.
				</p>
				<p>
					Use {num(meta?.rsa_good_bits)}-bit RSA or Ed25519; {num(meta?.rsa_min_bits)}-bit is a warning, smaller
					critical. Selectors can't be listed from DNS: Marcan learns them from reports; add others under
					<SettingsLink to="dkim-selectors">Settings → DNS checks</SettingsLink>.
				</p>
			{/snippet}
			{@render refSection('dkim', 'DKIM and selectors', dkim)}

			{#snippet rollout()}
				<ol class="list-decimal space-y-1 pl-5">
					<li><InlineCode>{'p=none'}</InlineCode>: collect reports, find every legitimate sender.</li>
					<li>Fix senders (SPF include or aligned DKIM) until delivered compliance is close to 100%.</li>
					<li>
						<InlineCode>{'p=quarantine'}</InlineCode>: failing mail goes to spam. <InlineCode>{'pct='}</InlineCode> can phase
						it in.
					</li>
					<li><InlineCode>{'p=reject'}</InlineCode>: failing mail is refused.</li>
				</ol>
				<p>
					A domain's page says when the next step is ready: {num(meta?.rate_pass_min_pct)}% delivered compliance or more
					over the last {num(meta?.readiness_window_days)} days, with reports under the current policy covering all of them,
					at least {num(meta?.readiness_min_messages)} messages under it, and at least
					{num(meta?.readiness_min_policy_share_pct)}% of the mail evaluated under it. Each step starts the count again.
				</p>
			{/snippet}
			{@render refSection('rollout', 'Rollout: none → quarantine → reject', rollout)}

			{#snippet score()}
				<p>
					Starts at 100, deducts once per distinct finding; {gradeBands}. Rates DNS only, not report data. <InlineCode
						>{'p=none'}</InlineCode
					> is critical, so a monitoring-only domain shows red until it enforces.
				</p>
				<p>
					Report data is rated apart from the score: delivered compliance, DKIM coverage and a sender's pass rate are
					green from {num(meta?.rate_pass_min_pct)}%, amber from {num(meta?.rate_warn_min_pct)}% and red below. A domain
					whose reports stop for {num(meta?.reports_stopped_days)} days is flagged (the domain warning under Settings → Mailbox).
				</p>
				<div class="overflow-x-auto rounded-md border">
					<table class="w-full text-xs">
						<tbody class="divide-y">
							{#each scoreRows as r (r.severity)}
								<tr>
									<td class="text-foreground px-3 py-1.5 font-medium">{r.severity}</td>
									<td class="px-3 py-1.5 text-right tabular-nums">{r.points}</td>
									<td class="px-3 py-1.5">{r.example}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/snippet}
			{@render refSection('score', 'How the score works', score)}

			{#snippet optional()}
				<ul class="list-disc space-y-1 pl-5">
					<li>
						<span class="text-foreground">MTA-STS</span>: requires TLS for inbound mail. No record or testing: {pts(
							'info'
						)}.
					</li>
					<li><span class="text-foreground">TLS-RPT</span>: daily TLS failure reports. No record: {pts('info')}.</li>
					<li><span class="text-foreground">BIMI</span>: brand logo in some inboxes. No score effect.</li>
				</ul>
			{/snippet}
			{@render refSection('optional', 'Optional: MTA-STS, TLS-RPT, BIMI', optional)}
		</div>
	</section>
</div>
