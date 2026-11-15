import { describe, expect, it } from 'vitest';
import { codeSpans } from './text';

describe('codeSpans', () => {
	it('splits a finding at its backtick spans', () => {
		expect(codeSpans('Publish `_mta-sts.a.example` TXT `v=STSv1; id=<unique>` and a policy.')).toEqual([
			{ code: false, text: 'Publish ' },
			{ code: true, text: '_mta-sts.a.example' },
			{ code: false, text: ' TXT ' },
			{ code: true, text: 'v=STSv1; id=<unique>' },
			{ code: false, text: ' and a policy.' }
		]);
	});

	it('keeps text without spans whole, and a span at either end', () => {
		expect(codeSpans('No record.')).toEqual([{ code: false, text: 'No record.' }]);
		expect(codeSpans('`p=reject`')).toEqual([{ code: true, text: 'p=reject' }]);
	});

	it('leaves an unpaired or empty backtick as written', () => {
		expect(codeSpans('a ` b')).toEqual([{ code: false, text: 'a ` b' }]);
		expect(codeSpans('a `` b')).toEqual([{ code: false, text: 'a `` b' }]);
	});
});
