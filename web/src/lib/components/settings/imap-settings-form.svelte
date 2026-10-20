<script lang="ts">
	import { resolve } from '$app/paths';
	import ReportingModeCards from '$lib/components/reporting-setup/reporting-mode-cards.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { plural, pluralNoun } from '$lib/format';
	import { statusText } from '$lib/tokens';
	import { looksLikeEmail } from '$lib/report-address';
	import { REPORT_DELIVERY_INTRO } from '$lib/reporting-modes';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import BusyButton from '../busy-button.svelte';
	import ImapConnectionFields from './imap-connection-fields.svelte';
	import { STOPPED_DAYS, WARN_HOURS, type ImapModel } from './imap-draft.svelte';
	import ImapFolderSelect from './imap-folder-select.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * The Mailbox page's sections: Connection (server, login, test), Reports (report address,
	 * folder, delivery) and Schedule, over the shared settings (the mailbox, reporting and
	 * monitoring groups), saved by the page's save bar; a changed connection or folder logs in
	 * first.
	 */
	let { model: m, idPrefix = 'imap' }: { model: ImapModel; idPrefix?: string } = $props();

	const form = $derived(m);
	joinPageForm(() => m);
	const draft = $derived(m.normalized);

	const id = (name: string) => `${idPrefix}-${name}`;
</script>

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

<SettingsPart {form} errorTitle="Mailbox settings failed to load" skeleton="h-96">
	{#snippet children(saved)}
		<SettingsSection title="Connection">
			<ImapConnectionFields model={m} {saved} {idPrefix} layout="settings" />

			<SettingRow title="Test">
				<div class="flex flex-wrap items-center gap-3">
					<BusyButton
						type="button"
						variant="outline"
						size="sm"
						busy={m.probing}
						busyLabel="Connecting…"
						disabled={!m.canTest}
						onclick={() => m.checkLogin(draft.folder)}
					>
						Test connection
					</BusyButton>
					{#if !m.probing && m.probe?.ok}
						<span class="text-status-pass-fg inline-flex items-center gap-1.5 text-xs">
							<CircleCheckIcon class="size-3.5 shrink-0" />
							{m.probe.message}
						</span>
					{:else if !m.probing && m.probe}
						<span class="text-status-fail-fg inline-flex min-w-0 items-start gap-1.5 text-xs" role="alert">
							<CircleAlertIcon class="mt-px size-3.5 shrink-0" />
							<span class="min-w-0 break-words">
								{m.probe.message}
								{#if m.probe.detail}
									<details class="mt-1">
										<summary class="cursor-pointer text-[11px] opacity-80">Details</summary>
										<span class="mt-1 block text-[11px]">{m.probe.detail}</span>
									</details>
								{/if}
							</span>
						</span>
					{/if}
				</div>
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="Reports">
			<SettingRow title="Report address" hint="The address in your domains' rua= tags. Empty means the login address.">
				<Input
					id={id('report-address')}
					type="email"
					bind:value={form.draft.reportAddress}
					placeholder={m.loginIsAddress ? draft.username : 'dmarc-reports@example.com'}
					aria-label="Report address"
					class="max-w-md text-sm"
				/>
				{#if draft.reportAddress && !looksLikeEmail(draft.reportAddress)}
					{@render note('Not a valid email address.', 'warn')}
				{:else if !m.loginIsAddress && draft.username && !draft.reportAddress}
					{@render note('The login is not an email address: enter the one reports go to.', 'warn')}
				{/if}
			</SettingRow>

			<SettingRow title="Folder">
				<ImapFolderSelect model={m} onpick={(name) => m.onFolderChange(name)} aria-label="Folder" class="max-w-md" />
				{#if m.folderStatus}
					{#if m.folderStatus.exists}
						{@render note(`${plural(m.folderStatus.messages ?? 0, 'message')} in this folder.`)}
					{:else}
						{@render note('Folder not found on the server.', 'warn')}
					{/if}
				{:else if !m.folders && !m.probing}
					{@render note('Test the connection to list the folders.')}
				{/if}
			</SettingRow>

			<SettingRow title="Delivery">
				{#snippet hint()}
					Changes what the
					<a href={resolve('/guide')} class="text-foreground underline underline-offset-2">Setup guide</a>
					asks you to publish.
				{/snippet}
				<p class="text-muted-foreground mb-3 text-xs leading-relaxed">{REPORT_DELIVERY_INTRO}</p>
				<ReportingModeCards
					bind:value={form.draft.mode}
					name="settings-reporting-mode"
					address={m.effectiveAddress}
					current={saved.reporting.reporting_mode_saved ? saved.reporting.reporting_mode : null}
				/>
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="Schedule">
			<SettingRow title="Checks">
				<label class="flex flex-wrap items-center gap-2 text-sm">
					Check the mailbox every
					<Input
						type="number"
						min="1"
						bind:value={form.draft.pollMinutes}
						aria-label="Minutes between checks"
						class="h-8 w-20 text-sm"
					/>
					{pluralNoun(draft.pollMinutes, 'minute')}
				</label>
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="Report arrival">
			<SettingRow title="Mailbox warning">
				<label class="flex flex-wrap items-center gap-2 text-sm">
					Warn when no reports at all have arrived for
					<select
						bind:value={form.draft.warnHours}
						aria-label="Hours without any report before the mailbox warns"
						class="border-input bg-background h-8 rounded-md border px-2 text-sm shadow-xs"
					>
						{#each WARN_HOURS as h (h)}
							<option value={h}>{h} hours</option>
						{/each}
						{#if !WARN_HOURS.includes(draft.warnHours)}
							<option value={draft.warnHours}>{plural(draft.warnHours, 'hour')}</option>
						{/if}
					</select>
				</label>
			</SettingRow>
			<SettingRow title="Domain warning">
				<label class="flex flex-wrap items-center gap-2 text-sm">
					Flag a domain after
					<select
						bind:value={form.draft.stoppedDays}
						aria-label="Days without reports before a domain is flagged"
						class="border-input bg-background h-8 rounded-md border px-2 text-sm shadow-xs"
					>
						{#each STOPPED_DAYS as d (d)}
							<option value={d}>{d} days</option>
						{/each}
						{#if !STOPPED_DAYS.includes(draft.stoppedDays)}
							<option value={draft.stoppedDays}>{plural(draft.stoppedDays, 'day')}</option>
						{/if}
					</select>
					without reports
				</label>
			</SettingRow>
		</SettingsSection>
	{/snippet}
</SettingsPart>
