<script lang="ts">
	import AccountCard from '$lib/components/settings/account-card.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SetupAccountForm from './setup-account-form.svelte';
	import type { SetupData } from './setup-steps';
	import StepNav from './step-nav.svelte';

	/**
	 * The accounts reports arrive in, one or more: each saved one as on Settings → Accounts, and
	 * a new one in the account form of Settings (`setup-account-form`). Its "Save and start
	 * import" tests the login and stores the account, whose import starts; then another can be
	 * added, or the setup moves on.
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

	const accounts = $derived(data.settings.accounts);
	// With no account yet the form is open from the start.
	// svelte-ignore state_referenced_locally
	let adding = $state(data.settings.accounts.length === 0);
	/** A new form per account added: a fresh model, a fresh draft. */
	let formKey = $state(0);
	/** The last account saved here, for the line above the cards. */
	let savedName = $state<string | null>(null);

	function addAnother() {
		formKey += 1;
		savedName = null;
		adding = true;
	}
</script>

<div class="space-y-4">
	<p class="text-muted-foreground text-xs leading-relaxed">
		The mailboxes that receive DMARC aggregate reports; a dedicated one works best. Marcan reads them over an encrypted
		connection without changing anything. Add one for each mailbox your domains' reports go to.
	</p>
	{#if accounts.length > 0}
		{#if savedName}
			<p class="text-xs" role="status">Saved {savedName}: its import starts now.</p>
		{/if}
		<div class="space-y-3">
			{#each accounts as account (account.id)}
				<AccountCard
					{account}
					health={data.health.accounts.find((h) => h.id === account.id)}
					sends={data.settings.sending.enabled && data.settings.sending.account_id === account.id}
				/>
			{/each}
		</div>
	{/if}
	{#if adding}
		<p class="text-xs leading-relaxed">
			Test the login, then pick the folder the reports land in. Nothing is imported before you save; then Marcan imports
			the reports already in that folder.
		</p>
		{#key formKey}
			<SetupAccountForm
				settings={() => data.settings}
				idPrefix={`setup-account-${formKey}`}
				onSaved={(name) => {
					savedName = name;
					adding = false;
				}}
				onCancel={accounts.length > 0 ? () => (adding = false) : undefined}
			/>
		{/key}
	{:else}
		<Button type="button" variant="outline" size="sm" onclick={addAnother}>
			<PlusIcon class="size-4" />
			Add another account
		</Button>
	{/if}
	<StepNav
		{onBack}
		onContinue={onNext}
		disabled={!done}
		hint={done ? undefined : 'Save an account that receives reports first.'}
	/>
</div>
