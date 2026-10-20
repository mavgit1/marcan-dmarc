<script lang="ts">
	import InlineCode from '$lib/components/inline-code.svelte';
	import ImapConnectForm from '$lib/components/settings/imap-connect-form.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import type { SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The report mailbox: Test connection lists the folders, "Save and start import" stores the
	 * connection and the folder together and starts the import, then the setup moves on.
	 */
	let {
		data,
		done,
		readonly = false,
		onBack,
		onNext
	}: {
		data: SetupData;
		done: boolean;
		readonly?: boolean;
		onBack: () => void;
		onNext: () => void;
	} = $props();

	const mailbox = $derived(data.settings.mailbox);
	const address = $derived(data.settings.reporting.report_address || mailbox.username);
</script>

<div class="space-y-4">
	{#if readonly}
		<p class="text-muted-foreground text-xs leading-relaxed">
			{#if mailbox.configured}
				Reports go to <InlineCode>{address}</InlineCode>; Marcan reads the folder <InlineCode
					>{mailbox.folder}</InlineCode
				>
				on <InlineCode>{mailbox.host}</InlineCode>.
			{:else}
				Not set up yet. Reports arrive daily as email attachments: use a dedicated mailbox, reachable over IMAP with TLS
				or STARTTLS.
			{/if}
			Change it under <SettingsLink to="mailbox">Settings → Mailbox</SettingsLink>.
		</p>
		<StepNav {onBack} label="Next" onContinue={onNext} />
	{:else}
		<p class="text-muted-foreground text-xs leading-relaxed">
			The IMAP mailbox that receives DMARC aggregate reports; a dedicated one works best. Marcan connects over TLS or
			STARTTLS, never unencrypted, and only reads it.
		</p>
		<p class="text-xs leading-relaxed">
			Test the connection, then pick the folder the reports land in. Nothing is imported before you save; then Marcan
			imports the reports already in that folder.
		</p>
		<ImapConnectForm
			settings={data.settings}
			idPrefix="setup-imap"
			onSaved={(saved) => {
				if (saved.reporting.report_address) onNext();
			}}
		/>
		<StepNav {onBack} onContinue={onNext} disabled={!done} hint={done ? undefined : 'Save the mailbox first.'} />
	{/if}
</div>
