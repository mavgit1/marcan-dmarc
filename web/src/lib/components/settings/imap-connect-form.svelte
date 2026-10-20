<script lang="ts">
	import type { SettingsResponse } from '$lib/types/SettingsResponse';
	import { Input } from '$lib/components/ui/input/index.js';
	import { plural } from '$lib/format';
	import { looksLikeEmail } from '$lib/report-address';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import Loader2Icon from '@lucide/svelte/icons/loader-2';
	import { untrack } from 'svelte';
	import BusyButton from '../busy-button.svelte';
	import { connectionKey, ImapModel } from './imap-draft.svelte';
	import ImapConnectionFields from './imap-connection-fields.svelte';
	import ImapFolderSelect from './imap-folder-select.svelte';

	/**
	 * The setup's connection (host, port, username, password, report address) and the
	 * folder to import. Test connection logs in and lists the folders; nothing is saved until
	 * Save and start import, which stores both together, so the import starts on the chosen
	 * folder. The typed password stays in the draft between the two.
	 */
	let {
		settings,
		idPrefix = 'imap',
		onSaved
	}: {
		/** The saved settings (`settings.current`). */
		settings: SettingsResponse | null;
		/** Keeps element ids unique when two forms could share a page. */
		idPrefix?: string;
		/** After a save, with what the server stored. */
		onSaved?: (saved: SettingsResponse) => void;
	} = $props();

	const m: ImapModel = new ImapModel({
		source: { value: () => settings },
		save: async (d, saved) => {
			const result = await m.saveConnection(d, saved);
			onSaved?.(result);
			return { saved: result, message: 'Saved, import started' };
		}
	});
	const form = m;
	const saved = $derived(m.saved);
	const draft = $derived(m.normalized);

	// A refused login is shown only while the connection fields still match it.
	let failure = $state.raw<{ key: string; message: string; detail: string | null } | null>(null);
	const connectError = $derived(failure?.key === connectionKey(draft) ? failure : null);

	async function testConnection() {
		const key = connectionKey(draft);
		failure = null;
		form.save.feedback = null;
		const res = await m.testConnection();
		if (res && !res.ok) failure = { key, message: res.message, detail: res.detail };
	}

	function pickFolder(name: string) {
		form.draft.folder = name;
		m.probe = null;
		form.save.feedback = null;
		void m.checkLogin(name);
	}

	/** The login is the report address when it is one, unless a different one is opened below. */
	let separateAddress = $state(false);
	/** Ask for a separate address only once a non-email login has been entered (not while typing). */
	let usernameLeft = $state(false);
	let seeded = false;
	$effect(() => {
		if (!saved || seeded) return;
		seeded = true;
		untrack(() => {
			separateAddress = looksLikeEmail(saved.mailbox.username) && draft.reportAddress !== '';
			usernameLeft = !!saved.mailbox.username;
		});
	});
	const usesAddressField = $derived(!m.loginIsAddress || separateAddress);
	// Once the address field is hidden (the login became an address), the login is the
	// report address: a value typed earlier must not be saved unseen.
	$effect(() => {
		if (seeded && !usesAddressField && form.draft.reportAddress !== '') form.draft.reportAddress = '';
	});
	const needsReportAddress = $derived(m.loginIsAddress ? separateAddress : usernameLeft && draft.username !== '');
	const reportAddressInvalid = $derived(usesAddressField && !looksLikeEmail(m.effectiveAddress));
	/** The saved mailbox is what the fields show. */
	const isSaved = $derived(!!saved?.mailbox.configured && saved.mailbox.has_password && !form.dirty);
	/** The folders of the login on screen are known: pick one, then save. */
	const listed = $derived(m.loginChecked && m.folders !== null);
	const canSave = $derived(listed && !m.probing && m.problem === null && m.folderStatus?.exists === true);

	/** Enter saves once the folders are listed, and tests the login before that. */
	function submit(e: SubmitEvent) {
		if (listed && !isSaved) return form.submit(e);
		e.preventDefault();
		if (m.problem === null && !m.probing) void testConnection();
	}

	function openSeparateAddress() {
		separateAddress = true;
		form.draft.reportAddress = draft.username;
	}
	function useLogin() {
		separateAddress = false;
		form.draft.reportAddress = '';
	}

	const id = (name: string) => `${idPrefix}-${name}`;
</script>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet differentAddress()}
	{#if m.loginIsAddress && !separateAddress}
		<button
			type="button"
			class="text-muted-foreground hover:text-foreground mt-1 text-[11px] underline underline-offset-2"
			onclick={openSeparateAddress}
		>
			Reports go to a different address than the login?
		</button>
	{/if}
{/snippet}

{#if saved}
	<form class="space-y-4" onsubmit={submit}>
		<ImapConnectionFields
			model={m}
			{saved}
			{idPrefix}
			layout="setup"
			onUsernameBlur={() => (usernameLeft = true)}
			usernameNote={differentAddress}
		/>

		{#if needsReportAddress}
			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<div>
					{@render label(id('report-address'), 'Report address')}
					<Input
						id={id('report-address')}
						name="report_address"
						type="email"
						bind:value={form.draft.reportAddress}
						placeholder="dmarc-reports@example.com"
						required
						aria-invalid={(form.draft.reportAddress.trim() !== '' && reportAddressInvalid) || undefined}
						aria-describedby={id('report-address-hint')}
						class="mt-1 text-sm"
					/>
					<p id={id('report-address-hint')} class="text-muted-foreground mt-1 text-[11px]">
						{#if form.draft.reportAddress.trim() !== '' && reportAddressInvalid}
							<span class="text-status-fail-fg">Not a valid email address.</span>
						{:else if m.loginIsAddress}
							The address your DMARC records send reports to.
							<button type="button" class="hover:text-foreground underline underline-offset-2" onclick={useLogin}>
								Use the login
							</button>
						{:else}
							The login is not an email address: enter the address this mailbox receives mail at.
						{/if}
					</p>
				</div>
			</div>
		{/if}

		{#if listed && m.folders}
			{@const countError = !m.probing && m.probe && !m.probe.ok ? m.probe.message : null}
			<div class="max-w-sm">
				{@render label(id('folder'), 'Folder with the reports')}
				<ImapFolderSelect
					model={m}
					onpick={pickFolder}
					id={id('folder')}
					name="folder"
					aria-describedby={id('folder-hint')}
					class="mt-1"
				/>
				<p id={id('folder-hint')} class="text-muted-foreground mt-1 text-[11px]">
					{#if m.probing}
						<Loader2Icon class="inline size-3 animate-spin" /> Counting messages…
					{:else if countError}
						<span class="text-status-fail-fg">{countError}</span>
					{:else if m.folderStatus}
						{#if m.folderStatus.exists}
							{plural(m.folderStatus.messages ?? 0, 'message')} in this folder. Marcan only reads it.
						{:else}
							<span class="text-status-fail-fg">Folder not found on the server.</span>
						{/if}
					{/if}
				</p>
			</div>
		{/if}

		{#if connectError}
			<div
				class="border-status-fail-border bg-status-fail-bg text-status-fail-fg flex items-start gap-2 rounded-md border p-3 text-xs"
				role="alert"
			>
				<CircleAlertIcon class="mt-0.5 size-4 shrink-0" />
				<div class="min-w-0 flex-1">
					<p class="font-medium">{connectError.message}</p>
					{#if connectError.detail}
						<details class="mt-1">
							<summary class="cursor-pointer text-[11px] opacity-80">Details</summary>
							<p class="mt-1 break-words text-[11px]">{connectError.detail}</p>
						</details>
					{/if}
				</div>
			</div>
		{/if}

		<div class="flex flex-wrap items-center justify-end gap-3 pt-1">
			{#if isSaved}
				<p class="text-status-pass-fg mr-auto flex items-center gap-1.5 text-xs font-medium" role="status">
					<CircleCheckIcon class="size-4 shrink-0" />
					<span>
						{form.save.feedback?.ok ? 'Saved, import started' : 'Saved'}: {saved.reporting.report_address ||
							saved.mailbox.username} · {saved.mailbox.folder}
					</span>
				</p>
			{:else if form.save.feedback && !form.save.feedback.ok}
				<p class="text-status-fail-fg mr-auto text-xs" role="alert">{form.save.feedback.message}</p>
			{/if}
			{#if listed && !isSaved}
				<BusyButton type="submit" size="sm" busy={form.save.busy} busyLabel="Saving…" disabled={!canSave}>
					Save and start import
				</BusyButton>
			{:else}
				<BusyButton
					type="submit"
					size="sm"
					variant={isSaved ? 'outline' : 'default'}
					busy={m.probing}
					busyLabel="Testing…"
					disabled={m.problem !== null}
				>
					{isSaved ? 'Test again' : 'Test connection'}
				</BusyButton>
			{/if}
		</div>
	</form>
{/if}
