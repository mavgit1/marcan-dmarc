<script lang="ts">
	import ImapConnectForm from '$lib/components/settings/imap-connect-form.svelte';
	import type { SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The report mailbox: Test connection lists the folders, "Save and start import" stores the
	 * connection and the folder together and starts the import, then the setup moves on.
	 */
	let {
		data,
		done,
		onBack,
		onNext
	}: {
		data: SetupData;
		done: boolean;
		onBack: () => void;
		onNext: () => void;
	} = $props();
</script>

<div class="space-y-4">
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
</div>
