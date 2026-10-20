<script lang="ts">
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Input } from '$lib/components/ui/input/index.js';
	import { getDnsCheck } from '$lib/api';
	import { CHECK_FAILED, dmarcPolicyVerdict, spfAllVerdict, type Verdict } from '$lib/dns';
	import { looksLikeEmail } from '$lib/report-address';
	import { resource } from '$lib/state/resource.svelte';
	import { statusText, statusTextPlain } from '$lib/tokens';
	import BusyButton from '../busy-button.svelte';
	import FeedbackAlert from './feedback-alert.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SecretField from './secret-field.svelte';
	import { ACCOUNT_CHANGED_HINT, SendingModel } from './sending-draft.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';
	import SecuritySelect from './security-select.svelte';
	import TestRecipientLine from './test-recipient-line.svelte';

	/**
	 * The Sending page's sections over the shared settings (the sending group): Server and login
	 * (the SMTP server, the login, the mailbox's by default), Sender (sender and Reply-To with the
	 * sender domain's SPF and DMARC, and the test recipient every test and preview goes to) and
	 * Test; saved by the page's save bar, under the page's
	 * switch (`SettingsPage`'s `feature`), which disables them while it is off.
	 */
	let { model: m, idPrefix = 'smtp' }: { model: SendingModel; idPrefix?: string } = $props();

	joinPageForm(() => m);
	const draft = $derived(m.normalized);

	/** The saved sender domain's live DNS check (cached on the server), for its SPF and DMARC. */
	const senderDns = resource({
		key: () => m.senderDomain ?? '',
		load: () => (m.senderDomain ? getDnsCheck(m.senderDomain) : null),
		refetchOnRefresh: false
	});
	const senderVerdicts = $derived.by((): { label: string; verdict: Verdict }[] | null => {
		if (!m.senderDomain) return null;
		const dns = senderDns.data;
		if (!dns) {
			if (!senderDns.error) return null;
			return [
				{ label: 'DMARC policy', verdict: CHECK_FAILED },
				{ label: 'SPF trailing all', verdict: CHECK_FAILED }
			];
		}
		return [
			{ label: 'DMARC policy', verdict: dmarcPolicyVerdict(dns) },
			{ label: 'SPF trailing all', verdict: spfAllVerdict(dns) }
		];
	});

	const id = (name: string) => `${idPrefix}-${name}`;
</script>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

<SettingsPart form={m} errorTitle="Sending settings failed to load" skeleton="h-72">
	{#snippet children(saved)}
		<SettingsSection title="Server and login">
			<SettingRow title="Server">
				<div class="grid gap-3 @sm:grid-cols-[minmax(0,1fr)_6rem] @lg:grid-cols-[minmax(0,1fr)_9rem_6rem]">
					<div class="min-w-0 @sm:col-span-2 @lg:col-span-1">
						{@render label(id('host'), 'Host')}
						<Input
							id={id('host')}
							bind:value={m.draft.host}
							placeholder="smtp.example.com"
							autocomplete="off"
							class="mt-1 font-mono text-sm"
						/>
					</div>
					<div>
						{@render label(id('security'), 'Security')}
						<SecuritySelect model={m} id={id('security')} class="mt-1" />
					</div>
					<div>
						{@render label(id('port'), 'Port')}
						<Input
							id={id('port')}
							type="number"
							min="1"
							max="65535"
							bind:value={m.draft.port}
							placeholder={String(m.defaultPort)}
							class="mt-1 text-sm"
						/>
					</div>
				</div>
			</SettingRow>

			<SettingRow title="Login">
				<label class="flex items-center gap-2 text-sm">
					<input type="checkbox" bind:checked={m.draft.same_login_as_mailbox} class="accent-primary size-4" />
					Same login as the mailbox
				</label>
				{#if !m.ownLogin}
					{#if saved.mailbox.username}
						{@render note(`Logs in as ${saved.mailbox.username} with the mailbox's saved password.`)}
					{:else}
						{@render note('The mailbox has no login yet.', 'warn')}
					{/if}
				{:else}
					<div class="mt-3 grid gap-3 @md:grid-cols-2">
						<div>
							{@render label(id('username'), 'Username')}
							<Input
								id={id('username')}
								bind:value={m.draft.username}
								placeholder="alerts@example.com"
								autocomplete="off"
								class="mt-1 text-sm"
							/>
						</div>
						<div>
							{@render label(id('password'), 'Password')}
							<SecretField
								id={id('password')}
								label="Password"
								bind:value={m.draft.password}
								stored={saved.sending.has_password && !m.accountChanged}
								clean={!m.dirty}
								placeholder={m.passwordRequired ? 'Enter password' : 'New password'}
								autocomplete="new-password"
								class="mt-1"
							/>
							{#if m.accountChanged && saved.sending.has_password}
								{@render note(ACCOUNT_CHANGED_HINT, 'warn')}
							{/if}
						</div>
					</div>
				{/if}
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="Sender">
			<SettingRow title="Sender address" hint="Its domain's SPF and DMARC decide whether the mail arrives.">
				<Input
					id={id('sender')}
					type="email"
					bind:value={m.draft.sender}
					placeholder={m.effectiveSender || 'dmarc@example.com'}
					aria-label="Sender address"
					class="max-w-md text-sm"
				/>
				{#if draft.sender && !looksLikeEmail(draft.sender)}
					{@render note('Not a valid email address.', 'warn')}
				{:else if !draft.sender && m.effectiveSender}
					{@render note(`Empty sends from ${m.effectiveSender}.`)}
				{/if}
				{#if m.senderDomain}
					<div class="mt-3 max-w-md space-y-1 text-xs">
						<p class="text-muted-foreground">Mail from <span class="font-mono">{m.senderDomain}</span>:</p>
						{#if senderVerdicts}
							<dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1">
								{#each senderVerdicts as row (row.label)}
									<dt class="text-muted-foreground">{row.label}</dt>
									<dd class={[statusTextPlain[row.verdict.tone], row.verdict.mono && 'font-mono']}>
										{row.verdict.label}
									</dd>
								{/each}
							</dl>
						{:else}
							<Skeleton class="h-4 w-48" />
							<Skeleton class="h-4 w-40" />
						{/if}
					</div>
				{/if}
			</SettingRow>

			<SettingRow title="Reply-To" hint="Empty: replies go to the sender address.">
				<Input
					id={id('reply-to')}
					type="email"
					bind:value={m.draft.reply_to}
					placeholder="help@example.com"
					aria-label="Reply-To address"
					class="max-w-md text-sm"
				/>
				{#if draft.reply_to && !looksLikeEmail(draft.reply_to)}
					{@render note('Not a valid email address.', 'warn')}
				{/if}
			</SettingRow>

			<SettingRow title="Test recipient" hint="Where test emails and previews go.">
				<Input
					id={id('test-recipient')}
					type="email"
					bind:value={m.draft.test_recipient}
					placeholder={m.effectiveSender || 'you@example.com'}
					aria-label="Test recipient"
					class="max-w-md text-sm"
				/>
				{#if draft.test_recipient && !looksLikeEmail(draft.test_recipient)}
					{@render note('Not a valid email address.', 'warn')}
				{:else if !draft.test_recipient && m.effectiveSender}
					{@render note(`Empty sends to ${m.effectiveSender}.`)}
				{/if}
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="Test">
			<SettingRow title="Test message">
				<BusyButton
					type="button"
					variant="outline"
					size="sm"
					busy={m.test.busy}
					busyLabel="Sending…"
					disabled={!!m.testBlocked}
					onclick={() => m.sendTest()}
				>
					Send a test
				</BusyButton>
				{#if m.testBlocked && !m.test.busy}
					{@render note(m.testBlocked)}
				{:else}
					<TestRecipientLine to={m.testRecipient} here />
				{/if}
				{#if m.test.feedback}
					<div class="mt-3 max-w-md">
						<FeedbackAlert feedback={m.test.feedback} failTitle="The test message was not sent" />
					</div>
				{/if}
			</SettingRow>
		</SettingsSection>
	{/snippet}
</SettingsPart>
