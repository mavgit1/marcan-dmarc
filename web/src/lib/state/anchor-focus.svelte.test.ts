// Links to a section (in Settings): each request shows its anchor once, a repeated link again, and a
// request made inside an effect (the page's hash) does not loop.
import { flushSync } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import { focusAnchor, focusUrlHash, onFocusAnchor } from './anchor-focus.svelte';

describe('anchor focus', () => {
	it('shows the asked-for anchor once per request', () => {
		vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
			cb(0);
			return 0;
		});
		const el = { scrollIntoView: vi.fn() } as unknown as HTMLElement;
		let shown = 0;
		const stop = $effect.root(() => {
			onFocusAnchor(
				() => 'section-x',
				() => el,
				() => shown++
			);
			$effect(() => {
				focusAnchor('section-x');
			});
		});
		flushSync();
		expect(shown).toBe(1);
		expect(el.scrollIntoView).toHaveBeenCalledTimes(1);

		focusAnchor('section-y');
		flushSync();
		expect(shown).toBe(1);

		focusAnchor('section-x');
		flushSync();
		expect(shown).toBe(2);
		// A URL's hash asks for its section.
		focusUrlHash('#section-x');
		flushSync();
		expect(shown).toBe(3);
		focusUrlHash('');
		flushSync();
		expect(shown).toBe(3);
		stop();
		vi.unstubAllGlobals();
	});
});
