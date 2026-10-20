/**
 * How SPF, DKIM, DMARC and row outcomes read in the UI: Failures tab, report records, the
 * domain overview and the export dialog. The words per server state are the tables below
 * (`CHECK_STATE_LABEL`, `DMARC_VERDICT_LABEL`) and `$lib/wording/outcome`; this composes them.
 *
 * DMARC only counts an *aligned* pass. SPF or DKIM can pass for another domain
 * (forwarding: SPF pass for gmail.com, DKIM pass for a list's domain) and still not
 * count, so a check is never shown as "failed" just because it did not carry DMARC.
 */
import type { CheckState } from '$lib/types/CheckState';
import type { DmarcVerdict } from '$lib/types/DmarcVerdict';
import type { Outcome } from '$lib/types/Outcome';
import type { OverrideReason } from '$lib/types/OverrideReason';
import type { StatusTone } from '$lib/tokens';
import { OUTCOME_LABEL, OUTCOME_STATUS, OUTCOME_TITLE } from '$lib/wording/outcome';

/**
 * One SPF or DKIM check of a record: the stored result and alignment, and the server's
 * `state` / `status` for it. Lists arrive most relevant first.
 */
export type AuthCheck = {
	domain: string | null;
	/** DKIM only. */
	selector?: string | null;
	/** Raw result (`pass`, `fail`, `softfail`, …); `null` when not reported. */
	result: string | null;
	state: CheckState;
	status: StatusTone;
};

export type AuthLabel = { label: string; tone: StatusTone; title: string };

/** How DMARC was decided for a record: label, colour and the explaining title. */
export const DMARC_VERDICT_LABEL = {
	fail: {
		label: 'DMARC fail',
		tone: 'fail',
		title: 'Neither SPF nor DKIM passed for a domain aligned with the From domain'
	},
	pass_dkim: {
		label: 'Pass via DKIM',
		tone: 'pass',
		title: 'DMARC passed: DKIM passed for an aligned domain'
	},
	pass_spf: {
		label: 'Pass via SPF',
		tone: 'pass',
		title: 'DMARC passed: SPF passed for an aligned domain'
	},
	pass_both: {
		label: 'DMARC pass',
		tone: 'pass',
		title: 'DMARC passed: SPF and DKIM both passed for aligned domains'
	}
} as const satisfies Record<DmarcVerdict, AuthLabel>;

/** What one SPF or DKIM check is worded from. */
export type CheckContext = {
	/** "SPF" or "DKIM". */
	mech: string;
	/** The checked domain (and selector), or "an unnamed domain". */
	name: string;
	/** The checked domain as reported, if any. */
	domain: string | null;
	/** The raw result, lower case (`softfail`, `permerror`, …). */
	raw: string;
};

/**
 * One check, in the server's `state`: "Pass" (passed for the From domain, counts for
 * DMARC), "Pass for <domain>" (another domain: a forwarder or an ESP's own domain),
 * "Temporary error" (the receiver's DNS lookup failed), "Fail" (softfail, permerror …
 * included; the raw result is in the title) or "None". The title explains alignment.
 */
export const CHECK_STATE_LABEL: Record<CheckState, (ctx: CheckContext) => { label: string; title: string }> = {
	pass: ({ mech, name }) => ({
		label: 'Pass',
		title: `${mech} passed for ${name}, aligned with the From domain: counts for DMARC`
	}),
	pass_other: ({ mech, name, domain }) => ({
		label: domain ? `Pass for ${domain}` : 'Pass for another domain',
		title: `${mech} passed for ${name}, which is not aligned with the From domain: does not count for DMARC`
	}),
	temporary: ({ mech, name }) => ({
		label: 'Temporary error',
		title: `${mech} temperror for ${name}: the receiver could not look up DNS at the time, so it is no verdict on the sender`
	}),
	fail: ({ mech, name, raw }) => ({ label: 'Fail', title: `${mech} ${raw} for ${name}` }),
	none: ({ mech, name, domain }) => ({
		label: 'None',
		title: domain ? `No ${mech} result for ${name}` : `No ${mech} result`
	})
};

/** Headers for per-sender aligned-pass counts and rates (`spf_pass_messages` / `dkim_pass_messages`). */
export const ALIGNED_PASS_LABEL = { spf: 'SPF aligned pass', dkim: 'DKIM aligned pass' } as const;

/**
 * One check, in the server's `state` and `status`: "Pass" (passed for the From domain,
 * counts for DMARC), "Pass for <domain>" (passed for another domain, e.g. a forwarder or an
 * ESP's own domain), "Temporary error" (`temperror`: the receiver's DNS lookup failed),
 * "Fail" (softfail, permerror … included; the raw result is in the title) or "None". The
 * title explains alignment for those who want the term.
 */
export function checkDisplay(check: AuthCheck, kind: 'spf' | 'dkim'): AuthLabel {
	const words = CHECK_STATE_LABEL[check.state]({
		mech: kind === 'spf' ? 'SPF' : 'DKIM',
		name: (check.domain || 'an unnamed domain') + (check.selector ? ` (selector ${check.selector})` : ''),
		domain: check.domain,
		raw: check.result?.trim().toLowerCase() ?? ''
	});
	return { ...words, tone: check.status };
}

/** The label of a record without any check of that kind. */
export function noCheckDisplay(kind: 'spf' | 'dkim'): AuthLabel {
	return {
		label: 'None',
		tone: 'neutral',
		title: kind === 'spf' ? 'No SPF result reported' : 'No DKIM signature reported'
	};
}

// ---------------------------------------------------------------------------
// Override reasons
// ---------------------------------------------------------------------------

const OVERRIDE_TYPE_NOTE: Record<string, string> = {
	forwarded: 'forwarded mail',
	trusted_forwarder: 'trusted forwarder',
	mailing_list: 'mailing list',
	sampled_out: 'outside pct sampling',
	local_policy: 'receiver’s own policy'
};

const OVERRIDE_COMMENT_NOTE: [RegExp, string][] = [
	[/^arc\s*=\s*pass$/i, 'trusted forwarding'],
	[/^arc\s*=\s*fail$/i, 'forwarding chain broken'],
	[/^sampled[_ ]?out$/i, 'outside pct sampling']
];

/** Override as `type: comment`, with a short gloss for well-known values: "local_policy: arc=pass (trusted forwarding)". */
export function overrideText(reason: OverrideReason): string {
	const type = reason.type?.trim() ?? '';
	const comment = reason.comment?.trim() ?? '';
	if (comment) {
		const note = OVERRIDE_COMMENT_NOTE.find(([re]) => re.test(comment))?.[1];
		const text = type ? `${type}: ${comment}` : comment;
		return note ? `${text} (${note})` : text;
	}
	const typeNote = OVERRIDE_TYPE_NOTE[type.toLowerCase()];
	return typeNote ? `${type} (${typeNote})` : type;
}

/** Overrides only matter when DMARC failed; on a pass they are a note with no effect. */
export const OVERRIDE_NO_EFFECT_TITLE = 'Receiver note, no effect: DMARC passed';

/**
 * Badge label, tone (`OUTCOME_STATUS`) and title of a row's outcome. The title says how
 * DMARC passed or which disposition blocked it, and carries the receiver's override notes.
 */
export function outcomeDisplay(
	outcome: Outcome,
	ctx: {
		verdict: DmarcVerdict;
		disposition: string | null | undefined;
		overrides: OverrideReason[] | null | undefined;
	}
): AuthLabel {
	const notes = (ctx.overrides ?? []).map(overrideText).filter(Boolean).join(', ');
	let title = OUTCOME_TITLE[outcome]({
		disposition: ctx.disposition,
		verdictTitle: DMARC_VERDICT_LABEL[ctx.verdict].title
	});
	if (notes) {
		title += outcome === 'passed' ? `\n${OVERRIDE_NO_EFFECT_TITLE}: ${notes}` : `\nReceiver note: ${notes}`;
	}
	return { label: OUTCOME_LABEL[outcome], tone: OUTCOME_STATUS[outcome], title };
}
