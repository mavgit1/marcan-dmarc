<script lang="ts">
	import type { SettingsResponse } from '$lib/types/SettingsResponse';
	import { Input } from '$lib/components/ui/input/index.js';
	import type { Snippet } from 'svelte';
	import { MAILBOX_CHANGED_HINT, type ImapModel } from './imap-draft.svelte';
	import SecretField from './secret-field.svelte';
	import SecuritySelect from './security-select.svelte';
	import SettingRow from './setting-row.svelte';

	/**
	 * The mailbox's server (host, security, port) and login (username, password), shared by the
	 * setup's form and the Mailbox settings page. `layout` picks the frame: `setup` lays them
	 * out in two grids of a plain form whose fields are required, `settings` in the page's
	 * Server and Login rows. The caller keeps the test, the folder and the save.
	 */
	let {
		model: m,
		saved,
		idPrefix,
		layout,
		onUsernameBlur,
		usernameNote,
		invalidField = null
	}: {
		model: ImapModel;
		/** The saved settings: whether a password is stored. */
		saved: SettingsResponse;
		idPrefix: string;
		layout: 'setup' | 'settings';
		onUsernameBlur?: () => void;
		/** Under the username (the setup's "different address" link). */
		usernameNote?: Snippet;
		/** The field marked invalid (`aria-invalid`): the one the disabled button's reason names. */
		invalidField?: string | null;
	} = $props();

	const form = $derived(m);
	const required = $derived(layout === 'setup');
	const id = (name: string) => `${idPrefix}-${name}`;
	/** The setup's fields carry their `name`, as a plain form's do. */
	const named = (name: string) => (required ? name : undefined);
	const invalid = (field: string) => invalidField === field || undefined;
</script>

{#snippet label(forId: string, text: string)}
	<label for={forId} class="text-muted-foreground text-xs font-medium">{text}</label>
{/snippet}

{#snippet host()}
	{@render label(id('host'), 'Host')}
	<Input
		id={id('host')}
		name={named('host')}
		bind:value={form.draft.host}
		aria-invalid={invalid('host')}
		placeholder="imap.example.com"
		autocomplete="off"
		{required}
		class="mt-1 font-mono text-sm"
	/>
{/snippet}

{#snippet security()}
	{@render label(id('security'), 'Security')}
	<SecuritySelect model={m} id={id('security')} class="mt-1" />
{/snippet}

{#snippet port()}
	{@render label(id('port'), 'Port')}
	<Input
		id={id('port')}
		name={named('port')}
		type="number"
		bind:value={form.draft.port}
		aria-invalid={invalid('port')}
		placeholder={String(m.defaultPort)}
		{required}
		class="mt-1 text-sm"
	/>
{/snippet}

{#snippet username()}
	{@render label(id('username'), 'Username')}
	<Input
		id={id('username')}
		name={named('username')}
		bind:value={form.draft.username}
		onblur={onUsernameBlur}
		aria-invalid={invalid('username')}
		placeholder="dmarc@example.com"
		autocomplete="username"
		{required}
		class="mt-1 text-sm"
	/>
	{@render usernameNote?.()}
{/snippet}

{#snippet password()}
	{@render label(id('password'), 'Password')}
	<SecretField
		id={id('password')}
		label="Password"
		bind:value={form.draft.password}
		stored={saved.mailbox.has_password && !m.mailboxChanged}
		clean={!form.dirty}
		invalid={invalidField === 'password'}
		placeholder={m.passwordRequired ? 'Enter password' : 'New password'}
		autocomplete="current-password"
		class="mt-1"
	/>
	{#if m.mailboxChanged && saved.mailbox.has_password}
		<p class="text-status-warn-fg mt-1 text-[11px]">{MAILBOX_CHANGED_HINT}</p>
	{/if}
{/snippet}

{#if layout === 'settings'}
	<SettingRow title="Server">
		<div class="grid gap-3 @sm:grid-cols-[minmax(0,1fr)_6rem] @lg:grid-cols-[minmax(0,1fr)_9rem_6rem]">
			<div class="min-w-0 @sm:col-span-2 @lg:col-span-1">{@render host()}</div>
			<div>{@render security()}</div>
			<div>{@render port()}</div>
		</div>
	</SettingRow>
	<SettingRow title="Login">
		<div class="grid gap-3 @md:grid-cols-2">
			<div>{@render username()}</div>
			<div>{@render password()}</div>
		</div>
	</SettingRow>
{:else}
	<div class="grid grid-cols-1 gap-4 sm:grid-cols-4">
		<div class="sm:col-span-2">{@render host()}</div>
		<div>{@render security()}</div>
		<div>{@render port()}</div>
	</div>
	<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
		<div>{@render username()}</div>
		<div>{@render password()}</div>
	</div>
{/if}
