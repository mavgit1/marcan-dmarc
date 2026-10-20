/** Semantic status class helpers; tokens and usage rules are in `src/lib/components/DESIGN.md`. */

import type { Severity } from '$lib/types/Severity';
import type { Status } from '$lib/types/Status';

/**
 * A verdict colour, as the server decides it (`Status`). `info` is blue = blocked by policy
 * (spoofing stopped). `forward` is violet = forwarded mail (failed in transit, receiver
 * trusted the forward): neither a warning nor a pass.
 */
export type StatusTone = Status;

/** Combined surface classes for badges and tinted rows. */
export const statusSurface: Record<StatusTone, string> = {
	pass: 'border-transparent bg-status-pass-bg text-status-pass-fg',
	fail: 'border-transparent bg-status-fail-bg text-status-fail-fg',
	warn: 'border-transparent bg-status-warn-bg text-status-warn-fg',
	neutral: 'border-transparent bg-status-neutral-bg text-status-neutral-fg',
	info: 'border-transparent bg-status-info-bg text-status-info-fg',
	forward: 'border-transparent bg-status-forward-bg text-status-forward-fg'
};

/** Tinted table-row background (no foreground — keep body text readable). */
export const statusRow: Record<StatusTone, string> = {
	pass: 'bg-status-pass-bg/60',
	fail: 'bg-status-fail-bg/70',
	warn: 'bg-status-warn-bg/70',
	neutral: 'bg-status-neutral-bg/60',
	info: 'bg-status-info-bg/60',
	forward: 'bg-status-forward-bg/60'
};

export const statusFg: Record<StatusTone, string> = {
	pass: 'text-status-pass-fg',
	fail: 'text-status-fail-fg',
	warn: 'text-status-warn-fg',
	neutral: 'text-status-neutral-fg',
	info: 'text-status-info-fg',
	forward: 'text-status-forward-fg'
};

/** Left accent border of a card that carries a verdict (scorecard pillars, two-sided readings). */
export const statusBorderL: Record<StatusTone, string> = {
	pass: 'border-l-status-pass',
	fail: 'border-l-status-fail',
	warn: 'border-l-status-warn',
	neutral: 'border-l-status-neutral',
	info: 'border-l-status-info',
	forward: 'border-l-status-forward'
};

/** Solid dot (status lights, legends). */
export const statusDot: Record<StatusTone, string> = {
	pass: 'bg-status-pass',
	fail: 'bg-status-fail',
	warn: 'bg-status-warn',
	neutral: 'bg-status-neutral',
	info: 'bg-status-info',
	forward: 'bg-status-forward'
};

/**
 * The one colour of each DNS finding severity (the server's verdict). `notice` is neutral on
 * purpose: zero impact on the score is not a pass. `info` is neutral too: blue means
 * spoofing stopped (DESIGN.md rule 2), and a minor finding is information.
 */
export const SEVERITY_TONE: Record<Severity, StatusTone> = {
	critical: 'fail',
	warning: 'warn',
	info: 'neutral',
	notice: 'neutral',
	ok: 'pass'
};

export const TONE_RANK: Record<StatusTone, number> = { fail: 5, warn: 4, forward: 3, info: 2, pass: 1, neutral: 0 };

/** A verdict colour said aloud (screen readers): the rate it stands next to is "protected", "critical", …. */
export const STATUS_WORD = {
	pass: 'protected',
	warn: 'warning',
	fail: 'critical',
	info: 'no verdict',
	forward: 'no verdict',
	neutral: 'no verdict'
} as const satisfies Record<Status, string>;

/** Row tint for a rated row: passing (and neutral) rows stay plain. */
export function rowTint(status: StatusTone): string | undefined {
	return status === 'pass' || status === 'neutral' ? undefined : statusRow[status];
}

const TONES: StatusTone[] = ['pass', 'fail', 'warn', 'neutral', 'info', 'forward'];
const byTone = (f: (tone: StatusTone) => string): Record<StatusTone, string> =>
	Object.fromEntries(TONES.map((t) => [t, f(t)])) as Record<StatusTone, string>;

/** Text that carries a verdict; a neutral one is a non-verdict and reads muted. */
export const statusText: Record<StatusTone, string> = byTone((t) =>
	t === 'neutral' ? 'text-muted-foreground' : statusFg[t]
);

/** `statusText` for the row's own statement: bold unless neutral. */
export const statusTextStrong: Record<StatusTone, string> = byTone((t) =>
	t === 'neutral' ? 'text-muted-foreground' : `font-medium ${statusFg[t]}`
);

/** Text that may carry a verdict; a neutral one is plain body text. */
export const statusTextPlain: Record<StatusTone, string> = byTone((t) =>
	t === 'neutral' ? 'text-foreground' : statusFg[t]
);

/** A rated figure in a table: a failing one is bold as well as red. */
export const statusFigure: Record<StatusTone, string> = byTone((t) =>
	t === 'fail' ? `${statusFg.fail} font-medium` : statusFg[t]
);
