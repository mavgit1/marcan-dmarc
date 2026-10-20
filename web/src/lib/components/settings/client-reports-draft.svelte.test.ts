// The client reports draft behind its Settings section: how the saved group becomes fields and
// back, what counts as a change, why a draft cannot be saved, and what the save and the preview
// send.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SENDING_ON, settingsFixture } from '$lib/test/settings-fixture';
import type { ClientReportSettingsResponse } from '$lib/types/ClientReportSettingsResponse';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import * as api from '$lib/api';
import type { ClientReportSettings } from '$lib/types/ClientReportSettings';
import { ClientReportsModel, dayProblem, replyToProblem } from './client-reports-draft.svelte';

vi.mock('$lib/api', () => ({
	getSettings: vi.fn(),
	saveClientReports: vi.fn(),
	sendClientReportEmail: vi.fn()
}));

/** The server's day range, as `GET /api/meta` serves it. */
const RANGE = { min: 1, max: 28 };
vi.mock('$lib/state/meta.svelte', () => ({
	meta: { current: { client_report_day_min: 1, client_report_day_max: 28 }, load: vi.fn() }
}));

const saveClientReports = vi.mocked(api.saveClientReports);
const sendClientReportEmail = vi.mocked(api.sendClientReportEmail);

/** Let promises settle and Svelte flush its effects (a macrotask later). */
const settle = () => new Promise<void>((done) => setTimeout(done));

function server(reports: Partial<ClientReportSettingsResponse> = {}, sendingOn = true): SettingsResponse {
	return settingsFixture(SENDING_ON, { sending: { enabled: sendingOn }, client_reports: reports });
}

const ON: Partial<ClientReportSettingsResponse> = {
	enabled: true,
	day_of_month: 15,
	hour_utc: 18,
	reply_to: 'help@msp.example'
};

/** How many recipients the list has, as the Recipients section would say. */
const recipients = $state({ count: 0 });

async function setup(saved: SettingsResponse | null) {
	let m!: ClientReportsModel;
	const stop = $effect.root(() => {
		m = new ClientReportsModel({ source: { value: () => saved }, recipientCount: () => recipients.count });
	});
	await settle();
	return { m, stop };
}

const draft = (over: Partial<ClientReportSettings> = {}): ClientReportSettings => ({
	enabled: false,
	day_of_month: 1,
	hour_utc: 7,
	reply_to: '',
	...over
});

beforeEach(() => {
	vi.clearAllMocks();
	recipients.count = 0;
});

describe('the draft and what is wrong with it', () => {
	it('is the saved group', async () => {
		const { m, stop } = await setup(server(ON));
		expect(m.draft).toEqual({ enabled: true, day_of_month: 15, hour_utc: 18, reply_to: 'help@msp.example' });
		stop();
	});

	it('is clean for the saved values written differently; a changed field is dirty', async () => {
		const { m, stop } = await setup(server(ON));
		const change = (over: Partial<ClientReportSettings>) => {
			m.discard();
			Object.assign(m.draft, over);
			return m.dirty;
		};
		expect(change({ reply_to: ' help@msp.example ' })).toBe(false);
		// A number field hands its value back as text.
		expect(change({ day_of_month: '15' as unknown as number })).toBe(false);
		expect(change({ day_of_month: 2 })).toBe(true);
		expect(change({ enabled: false })).toBe(true);
		stop();
	});

	it('names a Reply-To that is no address, and holds Save', async () => {
		expect(replyToProblem(draft())).toBeNull();
		const { m, stop } = await setup(server(ON));
		m.draft.reply_to = 'help';
		expect(m.problem).toBe('The Reply-To address help is not a valid email address.');
		expect(m.canSave).toBe(false);
		stop();
	});

	it("names a day of the month outside the server's range, like the server", () => {
		const why = 'The day of the month must be between 1 and 28, so every month has it.';
		expect(dayProblem(draft({ day_of_month: 1 }), RANGE)).toBeNull();
		expect(dayProblem(draft({ day_of_month: 28 }), RANGE)).toBeNull();
		expect(dayProblem(draft({ day_of_month: 29 }), RANGE)).toBe(why);
		expect(dayProblem(draft({ day_of_month: 0 }), RANGE)).toBe(why);
		expect(dayProblem(draft({ day_of_month: 2.5 }), RANGE)).toBe(why);
		// A cleared number field reads as 0 once normalized.
		expect(dayProblem(draft({ day_of_month: Number(null) }), RANGE)).toBe(why);
		// Until the range is known the server checks.
		expect(dayProblem(draft({ day_of_month: 29 }), null)).toBeNull();
	});
});

describe('ClientReportsModel', () => {
	it('the switch is the draft; switched on, Save waits until the list has a recipient', async () => {
		saveClientReports.mockResolvedValue(server(ON));
		const { m, stop } = await setup(server({ ...ON, enabled: false }));
		expect(m.feature).toMatchObject({ title: 'Send client reports', on: false });
		expect(m.blocked).toBeNull();
		m.feature!.set(true);
		await settle();
		expect(m.feature?.on).toBe(true);
		expect(m.dirty).toBe(true);
		expect(m.blocked).toBe('Add at least one recipient to the list first.');
		expect(saveClientReports).not.toHaveBeenCalled();
		recipients.count = 2;
		await settle();
		expect(m.blocked).toBeNull();
		m.submit();
		await settle();
		expect(saveClientReports).toHaveBeenCalledWith({
			enabled: true,
			day_of_month: 15,
			hour_utc: 18,
			reply_to: 'help@msp.example'
		});
		stop();
	});

	it('save() sends the whole group', async () => {
		saveClientReports.mockResolvedValue(server(ON));
		const { m, stop } = await setup(server());
		Object.assign(m.draft, { enabled: true, day_of_month: '15', hour_utc: '18', reply_to: ' help@msp.example ' });
		await settle();
		expect(m.dirty).toBe(true);
		m.submit();
		await settle();
		expect(saveClientReports).toHaveBeenCalledWith({
			enabled: true,
			day_of_month: 15,
			hour_utc: 18,
			reply_to: 'help@msp.example'
		});
		expect(m.save.feedback).toEqual({ ok: true, message: 'Saved' });
		stop();
	});

	it('save() refuses a day the input allowed to be typed out of range', async () => {
		const { m, stop } = await setup(server());
		Object.assign(m.draft, { day_of_month: 31 });
		await settle();
		expect(m.dayProblem).not.toBeNull();
		m.submit();
		await settle();
		expect(saveClientReports).not.toHaveBeenCalled();
		stop();
	});

	it('save() refuses an invalid draft before any request', async () => {
		const { m, stop } = await setup(server());
		m.draft.reply_to = 'help';
		await settle();
		m.submit();
		await settle();
		expect(saveClientReports).not.toHaveBeenCalled();
		expect(m.save.feedback).toEqual({
			ok: false,
			message: 'The Reply-To address help is not a valid email address.'
		});
		stop();
	});

	it('a preview goes to the test recipient, needs the switch and sending on, uses the Reply-To on screen, and answers under its organization', async () => {
		const off = await setup(server(ON, false));
		expect(off.m.testBlocked).toBe('Turn on sending to send a preview.');
		off.m.feature!.set(false);
		await settle();
		expect(off.m.testBlocked).toBe('Turn on client reports to send a preview.');
		off.stop();

		const nowhere = server(ON);
		nowhere.sending.effective_test_recipient = '';
		const none = await setup(nowhere);
		expect(none.m.testBlocked).toBe('Set a test recipient under Sending first.');
		none.stop();

		const elsewhere = server(ON);
		elsewhere.sending.effective_test_recipient = 'me@msp.example';
		const other = await setup(elsewhere);
		expect(other.m.testRecipient).toBe('me@msp.example');
		other.stop();

		const { m, stop } = await setup(server(ON));
		expect(m.testBlocked).toBeNull();
		expect(m.testRecipient).toBe('dmarc@example.com');
		m.draft.reply_to = 'desk';
		await settle();
		expect(m.testBlocked).toBe('The Reply-To address desk is not a valid email address.');
		m.draft.reply_to = ' desk@msp.example ';
		await settle();
		expect(m.testBlocked).toBeNull();

		sendClientReportEmail.mockResolvedValueOnce({
			ok: true,
			message: "Preview of Acme's report sent to dmarc@example.com."
		});
		await m.sendPreview(2);
		expect(sendClientReportEmail).toHaveBeenCalledWith({ organization_id: 2, reply_to: 'desk@msp.example' });
		expect(m.previewing).toBe(2);
		expect(m.test.feedback).toEqual({ ok: true, message: "Preview of Acme's report sent to dmarc@example.com." });
		sendClientReportEmail.mockRejectedValueOnce(new Error('boom'));
		await m.sendPreview(3);
		expect(m.previewing).toBe(3);
		expect(m.test.feedback?.ok).toBe(false);
		stop();
	});
});
