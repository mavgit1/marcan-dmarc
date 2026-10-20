// One SPF or DKIM check in plain words: "Pass", "Pass for <domain>", "Fail" (raw result in the title), "None".
import { describe, expect, it } from 'vitest';
import { CHECK_STATE_LABEL, checkDisplay, type CheckContext } from './auth-display';

const ctx: CheckContext = { mech: 'SPF', name: 'example.net', domain: 'example.net', raw: 'softfail' };

describe('CHECK_STATE_LABEL', () => {
	it('names the other domain a check passed for, when known', () => {
		expect(CHECK_STATE_LABEL.pass_other(ctx).label).toBe('Pass for example.net');
		expect(CHECK_STATE_LABEL.pass_other({ ...ctx, domain: null }).label).toBe('Pass for another domain');
	});

	it('keeps the raw result in the title of a failure', () => {
		expect(CHECK_STATE_LABEL.fail(ctx)).toEqual({ label: 'Fail', title: 'SPF softfail for example.net' });
	});
});

describe('checkDisplay', () => {
	it('words the check from its state and colours it with the server status', () => {
		const shown = checkDisplay(
			{ domain: 'example.net', selector: 's1', result: 'Pass', state: 'pass', status: 'pass' },
			'dkim'
		);
		expect(shown.label).toBe('Pass');
		expect(shown.tone).toBe('pass');
		expect(shown.title).toContain('DKIM passed for example.net (selector s1)');
	});
});
