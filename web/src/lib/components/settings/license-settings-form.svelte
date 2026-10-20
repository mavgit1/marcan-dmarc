<script lang="ts">
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
	 * The License page's form over the shared settings (the license group): what the license is
	 * (one sentence, with "Get a license" unless one is active), the license state (the server's
	 * `license.state` in its tone, who the key is licensed to, and "Manage subscription" and
	 * Branding while active), then the Operator key section: the key,
	 * never shown back once stored; the locked field reads the server's `key_hint` ("you@acme.example
	 * · …GPL7CQ") with Replace and Remove. A valid key switches the free-software notice off; every
	 * feature works without one.
	 */
	let { model: m }: { model: LicenseModel } = $props();
	joinPageForm(() => m);
	/** Where a license is bought and where its subscription is managed: the server's URLs. */
	const licenseUrl = $derived(meta.current?.license_url ?? null);
	const billingUrl = $derived(meta.current?.billing_url ?? null);
	$effect(() => {
		meta.load().catch(() => {});
	});
	const link = 'text-foreground underline underline-offset-2';
</script>

<SettingsPart form={m} errorTitle="License settings failed to load" skeleton="h-40">
	{#snippet children(saved)}
		{@const view = LICENSE_STATE[saved.license.state]}
		<p class="text-sm leading-relaxed">
			Marcan is free for any use. An operator license only replaces the free-software notice in client emails with your
			name, from USD 20 a month.
			{#if view.links === 'buy' && licenseUrl}
				<a href={licenseUrl} target="_blank" rel="noopener" class={link}>Get a license</a>
			{/if}
		</p>
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
					{#if view.links === 'manage'}
						<p class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
							{#if billingUrl}
								<a href={billingUrl} target="_blank" rel="noopener" class={link}>Manage subscription</a>
							{/if}
							<SettingsLink to="branding" />
						</p>
					{/if}
				</div>
			</div>
		</section>

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
