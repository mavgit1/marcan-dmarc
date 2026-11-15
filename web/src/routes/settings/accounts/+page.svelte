<script lang="ts">
	import { resolve } from '$app/paths';
	import AccountCard from '$lib/components/settings/account-card.svelte';
	import { ReceivingModel } from '$lib/components/settings/receiving-draft.svelte';
	import ReceivingSettingsForm from '$lib/components/settings/receiving-settings-form.svelte';
	import SettingsPage from '$lib/components/settings/settings-page.svelte';
	import SettingsPart from '$lib/components/settings/settings-part.svelte';
	import SettingsSection from '$lib/components/settings/settings-section.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { settings } from '$lib/state/settings.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';

	const m = new ReceivingModel({ source: { shared: settings } });
</script>

<SettingsPage page="accounts" hint="Mailboxes that receive reports and send mail." docs="accounts">
	<SettingsSection>
		<div class="space-y-3 px-5 py-4">
			<SettingsPart form={m} errorTitle="Accounts failed to load" skeleton="h-16">
				{#snippet children(s)}
					{#each s.accounts as account (account.id)}
						<AccountCard
							{account}
							health={systemHealth.current?.accounts.find((h) => h.id === account.id)}
							sends={s.sending.enabled && s.sending.account_id === account.id}
						/>
					{:else}
						<p class="text-muted-foreground text-sm">No account yet: add the mailbox your DMARC reports go to.</p>
					{/each}
				{/snippet}
			</SettingsPart>
			<Button variant="outline" size="sm" href={resolve('/settings/accounts/new')}>
				<PlusIcon class="size-4" />
				Add account
			</Button>
		</div>
	</SettingsSection>
	<ReceivingSettingsForm model={m} />
</SettingsPage>
