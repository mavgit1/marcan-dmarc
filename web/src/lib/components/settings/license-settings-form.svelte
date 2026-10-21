<script lang="ts">
	import { DOCS_URL } from '$lib/nav';
	import { meta } from '$lib/state/meta.svelte';
	import { statusBorderL, statusDot, statusText } from '$lib/tokens';
	import { cn } from '$lib/utils.js';
	import { LICENSE_STATE, type LicenseModel } from './license-draft.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SecretField from './secret-field.svelte';
	import SettingsLink from './settings-link.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * The License page's form over the shared settings (the license group): the one duty the license
	 * asks of an operator serving clients and that the alert and client report emails carry the
	 * notice (with Branding), the license state (the server's `license.state` in its tone, who the
	 * key is licensed to, and "Manage or cancel a subscription" and Branding while active), one line
	 * pointing at the docs' Pricing page (the plans and prices live there alone), then the Operator
	 * key section: the key, never shown back once stored; the locked field reads the server's
	 * `key_hint` ("you@acme.example · …GPL7CQ") with Replace and Remove. A valid key switches the
	 * free-software notice off; every feature works without one.
	 */
	let { model: m }: { model: LicenseModel } = $props();
	joinPageForm(() => m);
	/** Where the subscription behind a key is managed: the server's URL. */
	const billingUrl = $derived(meta.current?.billing_url ?? null);
	const pricingUrl = `${DOCS_URL}pricing`;
	$effect(() => {
		meta.load().catch(() => {});
	});
	const link = 'text-foreground underline underline-offset-2';
</script>

<SettingsPart form={m} errorTitle="License settings failed to load" skeleton="h-40">
	{#snippet children(saved)}
		{@const view = LICENSE_STATE[saved.license.state]}
		<div class="max-w-prose space-y-1.5 text-sm leading-relaxed">
			<p>
				Marcan is free software for any use. The license asks one thing of an operator who runs it for clients: tell
				each client that the service runs on software whose source code is freely available, that they may run it
				themselves, and where to get the source.
			</p>
			<p>
				The alert and client report emails (both experimental) carry the free-software notice that says so, unless a
				valid operator key is stored.
				<SettingsLink to="branding" />
			</p>
		</div>
		<section
			aria-label="License state"
			class={cn('bg-card rounded-xl border border-l-2 p-4', statusBorderL[view.tone])}
		>
			<div class="flex gap-2.5">
				<span class={cn('mt-1.5 size-2 shrink-0 rounded-full', statusDot[view.tone])} aria-hidden="true"></span>
				<div class="min-w-0 space-y-2">
					<p class="text-sm">
						<span class="font-medium">{view.title}</span>
						{#if saved.license.licensed_to}
							Licensed to <span class="font-medium break-words">{saved.license.licensed_to}</span>.
						{/if}
						{#if view.detail}{view.detail}{/if}
					</p>
					{#if saved.license.state === 'active'}
						<p class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
							{#if billingUrl}
								<a href={billingUrl} target="_blank" rel="noopener" class={link}>Manage or cancel a subscription</a>
							{/if}
							<SettingsLink to="branding" />
						</p>
					{/if}
				</div>
			</div>
		</section>

		<p class="text-sm">
			Plans and prices:
			<a href={pricingUrl} target="_blank" rel="noopener" class={link}>{pricingUrl.replace(/^https:\/\//, '')}</a>
		</p>

		<SettingsSection title="Operator key" anchor="operator-key">
			<SettingRow title="Key">
				<SecretField
					id="license-key"
					label="Operator key"
					what="key"
					bind:value={m.draft.key}
					stored={m.hasKey && !m.draft.remove}
					storedLabel={m.keyHint}
					clean={!m.dirty}
					placeholder="you@company.example.AbC…"
					autocomplete="off"
					onRemove={() => m.removeKey()}
					class="w-full max-w-md"
					inputClass="font-mono placeholder:font-sans"
				/>
				{#if m.draft.remove}
					<p class={['mt-1 text-[11px]', statusText.warn]}>The key is removed when you save.</p>
				{/if}
			</SettingRow>
		</SettingsSection>
	{/snippet}
</SettingsPart>
