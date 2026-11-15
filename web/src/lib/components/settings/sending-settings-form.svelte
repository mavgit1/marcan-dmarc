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
	import { noSenderAddress, SendingModel } from './sending-draft.svelte';
	import SettingRow from './setting-row.svelte';
	import { SELECT_CLASS } from './select-class';
	import SettingsLink from './settings-link.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';
	import TestRecipientLine from './test-recipient-line.svelte';

	/**
	 * The Sending page's sections over the shared settings (the sending group): Account (the one
	 * mail goes through and from, among the accounts that can send), Sender (the account's address
	 * with its domain's SPF and DMARC, Reply-To, and the test recipient every test and preview goes
	 * to) and Test; saved by the page's save bar, under the page's switch
	 * (`SettingsPage`'s `feature`), which disables them while it is off. The server and login are
	 * the account's (Settings → Accounts).
	 */
	let { model: m, idPrefix = 'smtp' }: { model: SendingModel; idPrefix?: string } = $props();

	joinPageForm(() => m);
	const draft = $derived(m.normalized);

	/** The sender domain's live DNS check (cached on the server), for its SPF and DMARC. */
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
				{ label: 'SPF ending', verdict: CHECK_FAILED }
			];
		}
		return [
			{ label: 'DMARC policy', verdict: dmarcPolicyVerdict(dns) },
			{ label: 'SPF ending', verdict: spfAllVerdict(dns) }
		];
	});

	const id = (name: string) => `${idPrefix}-${name}`;

	/** The chosen account has no address to send from: said at the sender, not again at the test. */
	const senderProblem = $derived(m.account && !m.sender ? noSenderAddress(m.account) : null);
</script>

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

<SettingsPart form={m} errorTitle="Sending settings failed to load" skeleton="h-72">
	<SettingsSection>
		<SettingRow title="Account" labelFor={m.accounts.length > 0 ? id('account') : undefined}>
			{#if m.accounts.length === 0}
				<p class="text-sm">
					No account can send yet. Add one with an SMTP server or a Microsoft login under
					<SettingsLink to="accounts">Accounts</SettingsLink>.
				</p>
			{:else}
				<select
					id={id('account')}
					bind:value={m.draft.account_id}
					aria-invalid={m.ariaInvalid('account_id')}
					class={[SELECT_CLASS, 'h-9 w-full max-w-md']}
				>
					<option value={null}>None chosen</option>
					{#each m.accounts as a (a.id)}
						<option value={a.id}>{a.name} ({a.smtp?.host ?? 'Microsoft Graph'})</option>
					{/each}
				</select>
				{#if m.fieldProblem('account_id')}
					{@render note(m.fieldProblem('account_id') ?? '', 'warn')}
				{:else if m.account && !m.account.ready_to_send}
					{@render note(m.account.credential.lock_text ?? `${m.account.name} has no complete login yet.`, 'warn')}
				{/if}
			{/if}
		</SettingRow>
	</SettingsSection>

	<SettingsSection title="Sender">
		{#if m.account}
			<SettingRow title="Sender address" hint="Its domain's SPF and DMARC decide whether the mail arrives.">
				{#if m.sender}
					<p class="max-w-md min-w-0 text-sm break-all">Sending as <span class="font-medium">{m.sender}</span></p>
				{:else}
					<p class={['text-sm', statusText.warn]}>
						{m.account.name} has no email address to send from. Enter one under
						<SettingsLink to="accounts">Accounts</SettingsLink>.
					</p>
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
		{/if}

		<SettingRow title="Reply-To">
			<Input
				id={id('reply-to')}
				type="email"
				bind:value={m.draft.reply_to}
				placeholder="Same as the sender address"
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
				placeholder="name@domain"
				aria-label="Test recipient"
				class="max-w-md text-sm"
			/>
			{#if draft.test_recipient && !looksLikeEmail(draft.test_recipient)}
				{@render note('Not a valid email address.', 'warn')}
			{:else if !draft.test_recipient && m.sender}
				{@render note(`Empty sends to ${m.sender}.`)}
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
				<!-- A missing account is said once, at the select; an account without an address, at the sender. -->
				{#if m.testBlocked !== m.fieldProblem('account_id') && m.testBlocked !== senderProblem}
					{@render note(m.testBlocked)}
				{/if}
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
</SettingsPart>
