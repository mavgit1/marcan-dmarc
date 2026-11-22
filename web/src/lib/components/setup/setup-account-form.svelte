<script lang="ts">
	import BusyButton from '#lib/components/busy-button.svelte';
	import { AccountModel } from '#lib/components/settings/account-draft.svelte.ts';
	import AccountForm from '#lib/components/settings/account-form.svelte';
	import FeedbackAlert from '#lib/components/settings/feedback-alert.svelte';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { statusText } from '#lib/tokens.ts';
	import type { SettingsResponse } from '#lib/types/SettingsResponse.ts';

	/**
	 * A new account in the setup: the account form of Settings → Accounts over its own model.
	 * "Save and start import" tests the login, stores the account (its import starts on its
	 * folder) and says so (`onSaved`). One instance per account added.
	 */
	let {
		settings,
		idPrefix,
		onSaved,
		onCancel
	}: {
		/** The saved settings (the setup's live copy). */
		settings: () => SettingsResponse;
		idPrefix: string;
		onSaved: (name: string) => void;
		/** Shown as Cancel while another account is there already. */
		onCancel?: () => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	const m = new AccountModel({ source: { value: settings }, id: null });

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		// A problem not said yet (its field was never typed in): say it, save nothing.
		if (m.problem !== null) return m.reveal();
		const result = await m.persist();
		if (result.ok && m.account) onSaved(m.account.name);
	}
</script>

<form class="space-y-4" aria-label="New account" onsubmit={submit}>
	<AccountForm model={m} {idPrefix} />
	<FeedbackAlert feedback={m.save.feedback?.ok ? null : m.save.feedback} failTitle="Not saved" />
	<div class="flex flex-wrap items-center justify-end gap-3">
		{#if m.dirty && m.held}
			<span class={['text-xs', statusText.warn]}>{m.held.message}</span>
		{/if}
		{#if onCancel}
			<Button type="button" variant="ghost" size="sm" onclick={onCancel}>Cancel</Button>
		{/if}
		{#if m.providerChosen}
			<BusyButton type="submit" size="sm" busy={m.save.busy} busyLabel="Saving…" disabled={!m.dirty || !!m.held}>
				Save and start import
			</BusyButton>
		{/if}
	</div>
</form>
