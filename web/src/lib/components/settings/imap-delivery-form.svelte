<script lang="ts">
	import type { SettingsResponse } from '$lib/types/SettingsResponse';
	import ReportingModeCards from '$lib/components/reporting-setup/reporting-mode-cards.svelte';
	import type { Snippet } from 'svelte';
	import BusyButton from '../busy-button.svelte';
	import FeedbackAlert from './feedback-alert.svelte';
	import { ImapModel } from './imap-draft.svelte';

	/** The setup's reporting mode: how the domains' reports reach the saved mailbox. */
	let {
		settings,
		saveLabel = 'Save',
		idPrefix = 'imap',
		help,
		actions,
		onSaved
	}: {
		/** The saved settings (`settings.current`). */
		settings: SettingsResponse | null;
		saveLabel?: string;
		/** Keeps element ids unique when two forms could share a page. */
		idPrefix?: string;
		/** Extra line under the mode cards (e.g. a guide link). */
		help?: Snippet;
		/** In place of the save button: the form's buttons, given whether it is saving (the setup's step navigation, its Continue a submit button). */
		actions?: Snippet<[busy: boolean]>;
		/** After a save, with what the server stored. */
		onSaved?: (saved: SettingsResponse) => void;
	} = $props();

	const m: ImapModel = new ImapModel({
		source: { value: () => settings },
		save: async (d) => {
			const result = await m.saveDelivery(d);
			onSaved?.(result);
			return { saved: result, message: 'Saved.' };
		}
	});
	const form = m;
	const saved = $derived(m.saved);

	const id = (name: string) => `${idPrefix}-${name}`;
</script>

{#if !saved?.mailbox.configured}
	<p class="text-muted-foreground text-xs">Connect the mailbox first; then choose how reports reach it.</p>
{:else}
	<form class="space-y-4" onsubmit={(e) => form.submit(e)}>
		<div class="space-y-2">
			<ReportingModeCards
				bind:value={form.draft.mode}
				name={id('reporting-mode')}
				address={saved.reporting.report_address}
				legend="How do reports reach this mailbox?"
			/>
			{#if help}
				<p class="text-muted-foreground text-[11px]">{@render help()}</p>
			{/if}
		</div>

		<FeedbackAlert feedback={form.save.feedback} failTitle="Save failed" />

		{#if actions}
			{@render actions(form.save.busy)}
		{:else}
			<div class="flex justify-end">
				<BusyButton type="submit" size="sm" busy={form.save.busy} busyLabel="Saving…">
					{saveLabel}
				</BusyButton>
			</div>
		{/if}
	</form>
{/if}
