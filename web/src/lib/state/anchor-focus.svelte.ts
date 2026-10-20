import { untrack } from 'svelte';

/**
 * A request to show one anchor (a Settings section or row such as the alert catalogue, a
 * reference section of /guide): a link to it was followed, or the page opened with its hash.
 * The element with that anchor unfolds and scrolls into view once it is on the page; `seq` lets
 * the same link work twice, and `done` keeps a section that mounts again later (after its part
 * loaded again) from jumping again.
 */
const focus = $state({ anchor: null as string | null, seq: 0, done: 0 });

/** Ask for `anchor` to be shown. Safe inside an effect: it reads nothing the effect would track. */
export function focusAnchor(anchor: string): void {
	untrack(() => {
		focus.anchor = anchor;
		focus.seq += 1;
	});
}

/** The page opened with `hash` (`#alert-kinds`, or empty): ask for its anchor. Call it in an effect over the URL. */
export function focusUrlHash(hash: string): void {
	const anchor = decodeURIComponent(hash.replace(/^#/, ''));
	if (anchor) focusAnchor(anchor);
}

/**
 * Call `show` (unfold, then scroll `element()` into view) whenever `anchor` is asked for and
 * the element is on the page. Create it during component initialisation (it owns an effect).
 */
export function onFocusAnchor(
	anchor: () => string | undefined,
	element: () => HTMLElement | null | undefined,
	show?: () => void
): void {
	$effect(() => {
		const seq = focus.seq;
		const el = element();
		if (!el || !anchor() || focus.anchor !== anchor() || focus.done === seq) return;
		untrack(() => {
			focus.done = seq;
			show?.();
			// After the unfolded content has laid out.
			requestAnimationFrame(() => el.scrollIntoView({ block: 'start' }));
		});
	});
}
