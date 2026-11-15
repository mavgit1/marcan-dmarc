// @vitest-environment happy-dom
// A failed action's banner ends with a "(docs)" link when the server's answer names the docs
// section on the fix (a Microsoft login's missing send role), and says the sentence alone otherwise.
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import FeedbackAlert from './feedback-alert.svelte';

const SENTENCE = 'The app registration may not send as dmarc@example.com. Try again later.';

describe('FeedbackAlert', () => {
	it('links a failure to its docs section', () => {
		const { container } = render(FeedbackAlert, {
			feedback: { ok: false, message: SENTENCE, docs: 'accounts#exchange-online' },
			failTitle: 'The test message was not sent'
		});
		expect(container.textContent).toContain('Try again later (docs).');
		const link = container.querySelector('a');
		expect(link?.getAttribute('href')).toBe('https://marcan.kaitiaki.tech/accounts#exchange-online');
	});

	it('says a failure without a docs section as it is', () => {
		const { container } = render(FeedbackAlert, { feedback: { ok: false, message: SENTENCE } });
		expect(container.textContent?.trim()).toBe(SENTENCE);
		expect(container.querySelector('a')).toBeNull();
	});
});
