<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { deleteAccount } from '#lib/api.ts';
	import ConfirmDialog from '#lib/components/confirm-dialog.svelte';
	import NotFoundState from '#lib/components/not-found-state.svelte';
	import { Button } from '#lib/components/ui/button/index.ts';
	import { pageMeta } from '#lib/state/page-meta.svelte.ts';
	import { settings } from '#lib/state/settings.svelte.ts';
	import { Action } from '../action.svelte';
	import { AccountModel } from './account-draft.svelte';
	import AccountForm from './account-form.svelte';
	import SettingsPage from './settings-page.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * An account's page (`/settings/accounts/{id}`) or a new one's (`/settings/accounts/new`): the
	 * account form with its save bar, then Delete (confirmed; the server refuses it while sending
	 * goes through the account, and says so). A new account, once saved, goes back to the list,
	 * unless it has a certificate to upload: then it stays, as the saved account.
	 */
	let { id }: { id: number | null } = $props();

	// The page is made per account: its route keys it by id.
	// svelte-ignore state_referenced_locally
	const m = new AccountModel({ source: { shared: settings }, id });
	const accounts = { href: resolve('/settings/accounts'), label: 'Accounts' };

	/** Deleted here: the page leaves, it never says "not found". */
	let removed = $state(false);
	/** An id the settings do not list (deleted elsewhere, or never there). */
	const missing = $derived(id !== null && settings.current !== null && m.account === null && !removed);

	$effect(() => {
		pageMeta.entityName = m.account?.name ?? null;
	});
	$effect(() => () => {
		pageMeta.entityName = null;
	});

	// Saved as a new account: back to the list, where its card shows it. One with a new certificate
	// stays, saved, for its Download .cer and Test: the certificate is uploaded after the save.
	$effect(() => {
		if (id === null && m.id !== null && !m.dirty && !m.storedCertificate) void goto(accounts.href);
	});

	let confirming = $state(false);
	const remove = new Action();

	function confirmDelete() {
		const account = m.account;
		if (!account) return;
		void remove.run(async () => {
			await settings.save(deleteAccount(account.id));
			removed = true;
			confirming = false;
			await goto(accounts.href);
		});
	}
</script>

{#if missing}
	<NotFoundState headline="Account not found" backHref={accounts.href} backLabel="Back to Accounts" />
{:else}
	<SettingsPage
		page="accounts"
		title={m.account?.name ?? (id === null ? 'New account' : 'Account')}
		back={accounts}
		hint="A mailbox Marcan reads reports from, or sends mail through."
		docs="accounts"
	>
		<AccountForm model={m} />
		{#snippet after()}
			{#if m.account}
				{#if confirming}
					<ConfirmDialog
						title="Delete {m.account.name}?"
						confirmLabel="Delete account"
						busyLabel="Deleting…"
						action={remove}
						onConfirm={confirmDelete}
						onCancel={() => {
							confirming = false;
							remove.feedback = null;
						}}
					>
						<p class="text-muted-foreground text-xs">Its login and receive state go; the reports it imported stay.</p>
					</ConfirmDialog>
				{:else}
					<SettingsSection>
						<div class="px-5 py-4">
							<Button type="button" variant="outline" size="sm" onclick={() => (confirming = true)}>
								Delete account
							</Button>
						</div>
					</SettingsSection>
				{/if}
			{/if}
		{/snippet}
	</SettingsPage>
{/if}
