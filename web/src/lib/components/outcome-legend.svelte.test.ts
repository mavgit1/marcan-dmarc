// @vitest-environment happy-dom
// The outcome legend as it renders: the four outcomes in order with their counts, a zero muted
// in place, the share in the title, the total apart, a trend only where given, and a skeleton
// before the counts.
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import '$lib/test/component-test.svelte';
import type { MailFigures } from '$lib/health';
import OutcomeLegend from './outcome-legend.svelte';

const mail: MailFigures = {
	total_messages: 1_000,
	compliant_messages: 900,
	blocked_messages: 60,
	forwarded_failures: 40,
	delivered_failures: 0
};

/** Each cell: its label, its count and whether the count is muted. */
function cells(container: HTMLElement) {
	return [...container.querySelectorAll('li')].map((li) => {
		const [label, figure] = li.querySelectorAll(':scope > span');
		const count = figure?.querySelector('span');
		return {
			label: label?.textContent?.trim(),
			count: count?.textContent?.trim(),
			muted: !!count?.classList.contains('text-muted-foreground'),
			title: li.getAttribute('title')
		};
	});
}

describe('OutcomeLegend', () => {
	it('shows the four outcomes in order with their counts, a zero muted in place', () => {
		const { container } = render(OutcomeLegend, { mail });
		const shown = cells(container);
		expect(shown.map((c) => c.label)).toEqual(['DMARC pass', 'Blocked by policy', 'Forwarded', 'Delivered failures']);
		expect(shown.map((c) => c.count)).toEqual(['900', '60', '40', '0']);
		expect(shown.map((c) => c.muted)).toEqual([false, false, false, true]);
		expect(shown[1].title).toMatch(/^6(\.0)?% of all mail\. /);
	});

	it('sets the total apart with its label, the words of a window when given', () => {
		const total = render(OutcomeLegend, { mail, total: true });
		expect(total.getByText('Messages total')).toBeTruthy();
		expect(total.getByText('1,000')).toBeTruthy();
		total.unmount();
		const report = render(OutcomeLegend, { mail, total: 'in this report' });
		expect(report.getByText('Messages in this report')).toBeTruthy();
	});

	it('shows a trend only where the server sent one', () => {
		const { container } = render(OutcomeLegend, {
			mail,
			trends: {
				compliant: { direction: 'up', delta: 12.5, status: 'pass' },
				blocked: null,
				forwarded: null,
				delivered: null
			}
		});
		const items = [...container.querySelectorAll('li')];
		expect(items[0].textContent).toContain('12.5');
		expect(items[1].textContent?.replace(/\s+/g, ' ').trim()).toBe('Blocked by policy 60');
	});

	it('is a skeleton of four cells before the counts', () => {
		const { container } = render(OutcomeLegend, { mail: null });
		const items = [...container.querySelectorAll('li')];
		expect(items).toHaveLength(4);
		expect(items.every((li) => li.getAttribute('aria-hidden') === 'true')).toBe(true);
	});
});
