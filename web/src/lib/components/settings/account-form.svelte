<script lang="ts">
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { accountCertificateUrl, draftCertificateUrl } from '$lib/api';
	import { formatDay, plural, relativeTime } from '$lib/format';
	import { DOCS_URL } from '$lib/nav';
	import { MAILBOX_LABEL } from '$lib/pipeline';
	import { meta } from '$lib/state/meta.svelte';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import { statusDot, statusText } from '$lib/tokens';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import XIcon from '@lucide/svelte/icons/x';
	import { tick } from 'svelte';
	import { Action } from '../action.svelte';
	import BusyButton from '../busy-button.svelte';
	import { AccountModel, forwardedField } from './account-draft.svelte';
	import AccountTestLines from './account-test-lines.svelte';
	import FeedbackAlert from './feedback-alert.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SecretField from './secret-field.svelte';
	import SecuritySelect from './security-select.svelte';
	import { SELECT_CLASS } from './select-class';
	import { SECURITY_LABEL } from './connection';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';
	import SwitchInput from './switch-input.svelte';

	/**
	 * One account's form (the Account page; the setup's account step). A new account starts with
	 * the provider's choice alone, nothing picked (each option with its hint and docs link; it
	 * decides the login and fills in the servers); the rest appears once one is picked, and the
	 * choice stays above it until the save. A stored account names its provider in a plain line:
	 * another provider means other servers and credentials, so another account. Rows in this
	 * order: Provider, Name, Email address, Login, Receive reports (with its folder and the
	 * addresses forwarded to it, one row each, added and removed by row), then Advanced (the
	 * servers, folded: the provider fills them) and Test (the last background check of a stored
	 * receiving account, as the health answer has it, its reason left out while a failed line of
	 * the test on screen says it; then one line per step the test ran).
	 * Saved by the page's save bar, which tests a changed login first. Advanced opens by itself
	 * where it has to be filled in (another provider without servers) or holds the save, and
	 * stays open once open: only its toggle folds it, so it never hides a field being typed in.
	 * A Microsoft 365 account has no servers, so no Advanced: its Login is the tenant and client
	 * ID and the certificate Marcan generates (thumbprint, expiry, the `.cer` to upload).
	 */
	let { model: m, idPrefix = 'account' }: { model: AccountModel; idPrefix?: string } = $props();

	joinPageForm(() => m);
	const draft = $derived(m.normalized);
	const id = (name: string) => `${idPrefix}-${name}`;

	/** Advanced is open: by its toggle, or by itself where it must be filled in; nothing else folds it. */
	let advancedShown = $state(false);
	/** Fields that live under Advanced. */
	const ADVANCED = new Set(['imapHost', 'imapPort', 'smtpPort']);
	/** Advanced has to be filled in: another provider without servers, or it holds the save. */
	const advancedNeeded = $derived(
		draft.loginKind === 'password' &&
			((m.saved !== null && !draft.imapHost && !draft.smtpHost && draft.provider === 'generic') ||
				ADVANCED.has(m.held?.field ?? ''))
	);
	$effect(() => {
		if (advancedNeeded) advancedShown = true;
	});

	/** Add an empty forwarded-address row and put the cursor in it. */
	async function addForwarded() {
		m.draft.forwardedFrom = [...m.draft.forwardedFrom, ''];
		await tick();
		document.getElementById(id(`forwarded-${m.draft.forwardedFrom.length - 1}`))?.focus();
	}

	function removeForwarded(index: number) {
		m.draft.forwardedFrom = m.draft.forwardedFrom.filter((_, i) => i !== index);
	}

	/** The certificate's "Generate": the server makes the key pair and keeps it until the save stores it. */
	const generate = new Action();

	$effect(() => {
		meta.load().catch(() => {});
	});
	/** No `MARCAN_SECRET_KEY`: the Login row says once that its credential (stored, or typed to be) is stored as is. */
	const unencrypted = $derived(meta.current?.secrets_encrypted === false);

	const serverLine = (host: string, port: number | null, security: 'tls' | 'starttls') =>
		host ? `${host}:${port ?? '—'} (${SECURITY_LABEL[security]})` : 'none';

	/** A stored receiving account's last background check (`undefined` while it does not receive). */
	const health = $derived(
		m.account?.receiving.enabled ? systemHealth.current?.accounts.find((h) => h.id === m.account?.id) : undefined
	);
	/** The lines of the test just run, while they are on screen under the Test button. */
	const testLines = $derived(m.testNote && !m.testing ? null : m.tested ? m.result : null);

	/**
	 * The last check's stored error, unless a failed line of the test on screen says it already (the
	 * same sentence, or that sentence with the server's answer in brackets after it): the
	 * failure is said once, and the status above keeps only its state word. The server's answer, when
	 * stored apart (`last_error_detail`), is folded under Details.
	 */
	const lastError = $derived.by(() => {
		const error = health?.last_error;
		if (!error) return null;
		const failed = testLines ? [testLines.token, testLines.mailbox, testLines.smtp] : [];
		return failed.some((line) => line && !line.ok && error.startsWith(line.message)) ? null : error;
	});
	const docsLink = 'text-foreground underline underline-offset-2';

	/** The draft field each control's id (after the prefix) edits, where the two differ. */
	const FIELD_OF: Record<string, string> = {
		tenant: 'tenantId',
		client: 'clientId',
		'imap-host': 'imapHost',
		'imap-port': 'imapPort',
		'smtp-port': 'smtpPort'
	};

	/** The control of this form an event came from, by its id after the prefix; `null` for another element. */
	function controlOf(e: Event): { key: string; value: string } | null {
		const el = e.target;
		if (!(el instanceof Element) || !el.id.startsWith(`${idPrefix}-`)) return null;
		return { key: el.id.slice(idPrefix.length + 1), value: 'value' in el ? String(el.value) : '' };
	}
</script>

<!--
	Leaving one of this form's fields counts its entries as changes (`AccountModel.touch`); a field
	typed in and left (the browser's `change`) says its problem from then on (`AccountModel.leave`).
-->
<svelte:document
	onfocusout={(e) => {
		if (controlOf(e)) m.touch();
	}}
	onchange={(e) => {
		const control = controlOf(e);
		if (!control) return;
		m.leave(
			control.key.startsWith('forwarded-') ? forwardedField(control.value) : (FIELD_OF[control.key] ?? control.key)
		);
	}}
/>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet note(text: string, tone: 'neutral' | 'warn' = 'neutral')}
	<p class={['mt-1 text-[11px]', statusText[tone]]}>{text}</p>
{/snippet}

{#snippet problem(field: string)}
	<!-- Said under the field once it was typed in and left, or Save was pressed; the save bar says it too. -->
	{@const text = m.edited ? m.fieldProblem(field) : null}
	{#if text}{@render note(text, 'warn')}{/if}
{/snippet}

<!-- These fields cannot be filled in without the guide: its steps happen in Entra and PowerShell. -->
{#snippet microsoftGuide()}
	Set up in Microsoft Entra and Exchange Online PowerShell: follow the
	<a href="{DOCS_URL}accounts#microsoft-365" target="_blank" rel="noreferrer" class={docsLink}>setup guide</a>.
{/snippet}

{#snippet providerDocs(docs: string)}
	(<a href="{DOCS_URL}{docs}" target="_blank" rel="noreferrer" class={docsLink}>docs</a>)
{/snippet}

<SettingsPart form={m} errorTitle="Account settings failed to load" skeleton="h-96">
	{#snippet children(saved)}
		{@const chosen = m.providerChosen}
		<SettingsSection>
			<SettingRow title="Provider" stacked={!chosen}>
				{#if m.account}
					<!-- Fixed once saved: another provider is another account. -->
					<p class="text-sm" data-testid="account-provider">{m.preset?.label ?? m.draft.provider}</p>
					{#if m.preset}
						<p class="text-muted-foreground mt-1 text-[11px]">{m.preset.hint} {@render providerDocs(m.preset.docs)}.</p>
					{/if}
				{:else}
					<div role="radiogroup" aria-label="Provider" class="grid gap-2 @2xl:grid-cols-3">
						{#each saved.presets as p (p.provider)}
							<label
								class={[
									'hover:bg-muted/40 flex min-w-0 cursor-pointer items-start gap-2.5 rounded-lg border px-3 py-2.5',
									chosen && m.draft.provider === p.provider && 'border-primary bg-muted/40'
								]}
							>
								<input
									type="radio"
									id={id(`provider-${p.provider}`)}
									name={id('provider')}
									value={p.provider}
									checked={chosen && m.draft.provider === p.provider}
									onchange={() => m.setProvider(p.provider)}
									class="accent-primary mt-0.5 size-4 shrink-0"
								/>
								<span class="min-w-0">
									<span class="block text-sm font-medium">{p.label}</span>
									<span class="text-muted-foreground block text-xs">{p.hint} {@render providerDocs(p.docs)}.</span>
								</span>
							</label>
						{/each}
					</div>
				{/if}
			</SettingRow>
			{#if chosen}
				<SettingRow title="Name" labelFor={id('name')}>
					<Input
						id={id('name')}
						bind:value={m.draft.name}
						aria-invalid={m.ariaInvalid('name')}
						autocomplete="off"
						class="max-w-md text-sm"
					/>
					{@render problem('name')}
				</SettingRow>

				<SettingRow title="Email address" hint="Where reports are sent, and the sender." labelFor={id('address')}>
					<Input
						id={id('address')}
						type="email"
						bind:value={() => m.draft.address, (v) => m.setAddress(v)}
						aria-invalid={m.ariaInvalid('address')}
						placeholder="name@domain"
						autocomplete="off"
						class="max-w-md text-sm"
					/>
					{@render problem('address')}
				</SettingRow>

				<SettingRow title="Login" hint={m.draft.loginKind === 'microsoft' ? microsoftGuide : undefined}>
					{#if m.draft.loginKind === 'password'}
						<div class="grid gap-3 @md:grid-cols-2">
							<div class="min-w-0">
								{@render label(id('username'), 'Username')}
								<Input
									id={id('username')}
									bind:value={m.draft.username}
									aria-invalid={m.ariaInvalid('username')}
									placeholder="name@domain"
									autocomplete="off"
									class="mt-1 text-sm"
								/>
							</div>
							<div class="min-w-0">
								{@render label(id('password'), 'Password')}
								<SecretField
									id={id('password')}
									label="Password"
									bind:value={m.draft.password}
									stored={m.passwordStored}
									clean={!m.dirty}
									invalid={!!m.ariaInvalid('password')}
									autocomplete="new-password"
									class="mt-1"
								/>
							</div>
						</div>
						{@render problem('username')}
						{@render problem('password')}
					{:else}
						<div class="grid gap-3 @md:grid-cols-2">
							<div class="min-w-0">
								{@render label(id('tenant'), 'Tenant ID')}
								<Input
									id={id('tenant')}
									bind:value={m.draft.tenantId}
									aria-invalid={m.ariaInvalid('tenantId')}
									placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
									autocomplete="off"
									spellcheck="false"
									class="mt-1 font-mono text-sm"
								/>
							</div>
							<div class="min-w-0">
								{@render label(id('client'), 'Client ID')}
								<Input
									id={id('client')}
									bind:value={m.draft.clientId}
									aria-invalid={m.ariaInvalid('clientId')}
									placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
									autocomplete="off"
									spellcheck="false"
									class="mt-1 font-mono text-sm"
								/>
							</div>
						</div>
						{@render problem('tenantId')}
						{@render problem('clientId')}
						<!-- The certificate on screen: the one just generated, else the stored one. -->
						{@const shown = m.draft.certificate ?? m.storedCertificate}
						{@const download = m.draft.certificate
							? draftCertificateUrl(m.draft.certificate.draft_id)
							: m.storedCertificate && m.account
								? accountCertificateUrl(m.account.id)
								: null}
						<div class="mt-3 flex flex-wrap items-center gap-2">
							<BusyButton
								type="button"
								variant="outline"
								size="sm"
								busy={generate.busy}
								busyLabel="Generating…"
								onclick={() => generate.run(async () => void (await m.generateCertificate()))}
							>
								{shown ? 'Generate new' : 'Generate'}
							</BusyButton>
							{#if download}
								<Button variant="ghost" size="sm" href={download} download>Download .cer</Button>
							{/if}
						</div>
						{#if shown}
							<dl
								class="text-muted-foreground mt-2 grid max-w-md grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs"
							>
								<dt>Thumbprint</dt>
								<dd class="text-foreground font-mono break-all" data-testid="certificate-thumbprint">
									{shown.thumbprint}
								</dd>
								{#if shown.expires_at !== null}
									<dt>Expires</dt>
									<dd class="text-foreground">{formatDay(shown.expires_at)}</dd>
								{/if}
							</dl>
							{#if m.draft.certificate}
								{@render note(
									"Save, then upload the .cer to the app registration (Certificates & secrets, Certificates). Test after the guide's Exchange Online step."
								)}
							{/if}
						{/if}
						{@render problem('certificate')}
						{#if generate.feedback && !generate.feedback.ok}
							<div class="mt-2 max-w-md">
								<FeedbackAlert feedback={generate.feedback} />
							</div>
						{/if}
					{/if}
					{#if unencrypted && m.credentialPresent}
						<p class="text-muted-foreground mt-2 text-[11px]">
							Not encrypted (<a
								href="{DOCS_URL}security#secrets-at-rest"
								target="_blank"
								rel="noreferrer"
								class={docsLink}>docs</a
							>)
						</p>
					{/if}
				</SettingRow>

				<SettingRow title="Receive reports" labelFor={id('receive')}>
					<SwitchInput id={id('receive')} checked={m.draft.receive} onchange={(on) => (m.draft.receive = on)} />
					{#if m.draft.receive && m.account?.shared_mailbox}
						{@render note(m.account.shared_mailbox, 'warn')}
					{/if}
					{#if m.draft.receive}
						<div class="mt-3 max-w-md space-y-3">
							<div>
								{@render label(id('folder'), 'Folder')}
								<select
									id={id('folder')}
									value={m.draft.folder}
									onchange={(e) => m.pickFolder(e.currentTarget.value)}
									class={[SELECT_CLASS, 'mt-1 h-9 w-full']}
								>
									{#if !m.folders?.some((f) => f.name === m.draft.folder)}
										<option value={m.draft.folder}>{m.draft.folder || saved.account_defaults.folder}</option>
									{/if}
									{#each m.folders ?? [] as f (f.name)}
										<option value={f.name}>{f.name}</option>
									{/each}
								</select>
								{#if m.folderStatus}
									{#if m.folderStatus.exists}
										{@render note(`${plural(m.folderStatus.messages ?? 0, 'message')} in this folder.`)}
									{:else}
										{@render note('Folder not found on the server.', 'warn')}
									{/if}
								{:else if !m.folders && !m.testing}
									{@render note('Test the login to list the folders.')}
								{/if}
							</div>
							{#if m.draft.forwardedFrom.length > 0}
								<div role="group" aria-labelledby={id('forwarded-label')}>
									<p id={id('forwarded-label')} class="text-muted-foreground text-xs font-medium">Forwarded from</p>
									<ul class="mt-1 space-y-2">
										{#each m.draft.forwardedFrom as address, i (i)}
											{@const field = forwardedField(address)}
											<li>
												<div class="flex items-center gap-1">
													<Input
														id={id(`forwarded-${i}`)}
														type="email"
														bind:value={m.draft.forwardedFrom[i]}
														aria-label="Forwarded from, address {i + 1}"
														aria-invalid={address.trim() ? m.ariaInvalid(field) : undefined}
														placeholder="name@domain"
														autocomplete="off"
														class="text-sm"
													/>
													<Button
														variant="ghost"
														size="icon-sm"
														class="text-muted-foreground hover:text-foreground shrink-0"
														aria-label={address.trim() ? `Remove ${address.trim()}` : 'Remove this row'}
														onclick={() => removeForwarded(i)}
													>
														<XIcon />
													</Button>
												</div>
												<!-- Said once, under the last row with that address (a repeated one marks both). -->
												{#if address.trim() && !m.draft.forwardedFrom
														.slice(i + 1)
														.some((f) => forwardedField(f) === field)}
													{@render problem(field)}
												{/if}
											</li>
										{/each}
									</ul>
									<button
										type="button"
										class="text-muted-foreground hover:text-foreground mt-2 text-xs underline underline-offset-2"
										onclick={addForwarded}
									>
										Add another address
									</button>
								</div>
							{:else}
								<button
									type="button"
									class="text-muted-foreground hover:text-foreground text-xs underline underline-offset-2"
									onclick={addForwarded}
								>
									Reports forwarded from other addresses?
								</button>
							{/if}
						</div>
					{/if}
				</SettingRow>
			{/if}
		</SettingsSection>

		{#if chosen && m.draft.loginKind === 'password'}
			<SettingsSection title="Advanced">
				<div class="min-w-0 px-5 py-4">
					<button
						type="button"
						class="hover:text-foreground inline-flex min-w-0 items-start gap-1.5 text-left text-sm"
						aria-expanded={advancedShown}
						aria-controls={id('advanced')}
						onclick={() => (advancedShown = !advancedShown)}
					>
						<ChevronRightIcon
							class={[
								'text-muted-foreground mt-0.5 size-4 shrink-0 transition-transform',
								advancedShown && 'rotate-90'
							]}
						/>
						<span class="min-w-0 break-all">
							IMAP {serverLine(draft.imapHost, draft.imapPort, draft.imapSecurity)} · SMTP {serverLine(
								draft.smtpHost,
								draft.smtpPort,
								draft.smtpSecurity
							)}
						</span>
					</button>
				</div>
				<div id={id('advanced')} class={['divide-border divide-y border-t', !advancedShown && 'hidden']}>
					{#each ['imap', 'smtp'] as const as proto (proto)}
						<SettingRow title={proto === 'imap' ? 'IMAP server' : 'SMTP server'}>
							<div class="grid gap-3 @sm:grid-cols-[minmax(0,1fr)_6rem] @lg:grid-cols-[minmax(0,1fr)_9rem_6rem]">
								<div class="min-w-0 @sm:col-span-2 @lg:col-span-1">
									{@render label(id(`${proto}-host`), 'Host')}
									{#if proto === 'imap'}
										<Input
											id={id('imap-host')}
											bind:value={m.draft.imapHost}
											aria-invalid={m.ariaInvalid('imapHost')}
											placeholder="imap.provider.tld"
											autocomplete="off"
											class="mt-1 font-mono text-sm"
										/>
									{:else}
										<Input
											id={id('smtp-host')}
											bind:value={m.draft.smtpHost}
											placeholder="smtp.provider.tld"
											autocomplete="off"
											class="mt-1 font-mono text-sm"
										/>
									{/if}
								</div>
								<div>
									{@render label(id(`${proto}-security`), 'Security')}
									{#if proto === 'imap'}
										<SecuritySelect
											id={id('imap-security')}
											value={m.draft.imapSecurity}
											onchange={(s) => m.setImapSecurity(s)}
											class="mt-1"
										/>
									{:else}
										<SecuritySelect
											id={id('smtp-security')}
											value={m.draft.smtpSecurity}
											onchange={(s) => m.setSmtpSecurity(s)}
											class="mt-1"
										/>
									{/if}
								</div>
								<div>
									{@render label(id(`${proto}-port`), 'Port')}
									{#if proto === 'imap'}
										<Input
											id={id('imap-port')}
											type="number"
											min="1"
											max="65535"
											bind:value={m.draft.imapPort}
											aria-invalid={m.ariaInvalid('imapPort')}
											class="mt-1 text-sm"
										/>
									{:else}
										<Input
											id={id('smtp-port')}
											type="number"
											min="1"
											max="65535"
											bind:value={m.draft.smtpPort}
											aria-invalid={m.ariaInvalid('smtpPort')}
											class="mt-1 text-sm"
										/>
									{/if}
								</div>
							</div>
							{#if proto === 'imap'}
								{@render problem('imapHost')}
								{@render problem('imapPort')}
							{:else}
								{@render problem('smtpPort')}
								{#if !draft.smtpHost && draft.loginKind === 'password'}{@render note(
										'Without an SMTP server the account cannot send.'
									)}{/if}
							{/if}
						</SettingRow>
					{/each}
				</div>
			</SettingsSection>
		{/if}

		{#if chosen}
			<SettingsSection title="Test">
				<div class="@container min-w-0 px-5 py-4 text-sm">
					{#if health}
						<!-- The last background check, as the account's card says it, with its reason. -->
						<div class="mb-3 max-w-md space-y-0.5 text-xs" data-testid="account-last-check">
							<p class="text-muted-foreground flex items-center gap-1.5">
								<span class={['size-1.5 shrink-0 rounded-full', statusDot[health.status]]}></span>
								{MAILBOX_LABEL[health.state]}{health.last_check_at !== null
									? `, checked ${relativeTime(health.last_check_at)}`
									: ''}
							</p>
							{#if lastError}
								<p class={['break-words', statusText.fail]}>{lastError}</p>
								{#if health.last_error_detail}
									<details class={statusText.fail}>
										<summary class="cursor-pointer text-[11px] opacity-80">Details</summary>
										<span class="mt-1 block text-[11px] break-all">{health.last_error_detail}</span>
									</details>
								{/if}
							{/if}
						</div>
					{/if}
					<BusyButton
						type="button"
						variant="outline"
						size="sm"
						busy={m.testing}
						busyLabel="Testing…"
						disabled={!!m.testProblem}
						onclick={() => m.testLogin()}
					>
						Test
					</BusyButton>
					{#if m.testNote && !m.testing}
						{@render note(m.testNote)}
					{:else if testLines}
						<AccountTestLines result={testLines} login={draft.loginKind} />
					{/if}
				</div>
			</SettingsSection>
		{/if}
	{/snippet}
</SettingsPart>
