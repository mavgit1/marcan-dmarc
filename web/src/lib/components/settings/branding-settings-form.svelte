<script lang="ts">
	import Skeleton from '$lib/components/skeleton.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { meta } from '$lib/state/meta.svelte';
	import { statusText } from '$lib/tokens';
	import { type BrandingModel, LOGO_ACCEPT, LOGO_TYPE_LABEL, kilobytes, previewNote } from './branding-draft.svelte';
	import EmailLookCard from './email-look-card.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SettingsLink from './settings-link.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * The Branding page's section over the shared settings (the branding group): the operator
	 * name and the logo, an uploaded image (picked here, sent with the page's save bar, previewed
	 * small), then "How your emails look": the draft's email as it goes out now (From name, top,
	 * title and last line, all the server's: `m.preview`, the code the emails use), one line on
	 * what the license changes, and the same email in the other license state under it. Name and
	 * logo are for the emails only; the UI carries no operator branding anywhere else.
	 */
	let { model: m }: { model: BrandingModel } = $props();
	joinPageForm(() => m);
	const logo = $derived(m.draft.logo);
	let fileInput = $state<HTMLInputElement | null>(null);

	/** A preview of the picked image, revoked when it changes. */
	let pickedUrl = $state<string | null>(null);
	$effect(() => {
		const file = logo.kind === 'replace' ? logo.file : null;
		if (!file) {
			pickedUrl = null;
			return;
		}
		const url = URL.createObjectURL(file);
		pickedUrl = url;
		return () => URL.revokeObjectURL(url);
	});
	// A saved or discarded draft forgets the picked file, so the picker does too.
	$effect(() => {
		if (logo.kind !== 'replace' && fileInput) fileInput.value = '';
	});

	/** The draft's logo: the picked image, else the stored one. */
	const logoUrl = $derived(
		logo.kind === 'replace' ? pickedUrl : logo.kind === 'keep' ? (meta.current?.branding.logo ?? null) : null
	);
	const emails = $derived(m.preview);
</script>

<SettingsPart form={m} errorTitle="Branding settings failed to load" skeleton="h-40">
	{#snippet children(saved)}
		<SettingsSection>
			<SettingRow title="Operator name" hint="Also the sender name on emails.">
				<Input
					id="branding-name"
					bind:value={m.draft.operator_name}
					placeholder="Acme IT"
					maxlength={meta.current?.operator_name_max_chars}
					aria-label="Operator name"
					class="max-w-md min-w-0 text-sm"
				/>
			</SettingRow>

			<SettingRow
				title="Logo"
				hint="PNG, JPEG or WebP, up to {meta.current
					? kilobytes(meta.current.logo_max_bytes)
					: '…'}. Wide logos read best."
			>
				<div class="flex min-w-0 flex-wrap items-center gap-3">
					{#if logoUrl}
						<img
							src={logoUrl}
							alt="Logo preview"
							class="bg-muted/40 ring-foreground/10 max-h-10 max-w-40 rounded object-contain p-1 ring-1"
						/>
					{/if}
					<input
						bind:this={fileInput}
						id="branding-logo"
						type="file"
						accept={LOGO_ACCEPT}
						aria-label="Logo image"
						class="file:bg-muted file:text-foreground hover:file:bg-muted/70 min-w-0 max-w-full text-xs file:mr-3 file:rounded-md file:border-0 file:px-3 file:py-1.5 file:text-xs file:font-medium"
						onchange={(e) => m.pick(e.currentTarget.files?.[0])}
					/>
					{#if logo.kind === 'replace' || (logo.kind === 'keep' && saved.branding.has_logo)}
						<Button type="button" variant="ghost" size="sm" onclick={() => m.removeLogo()}>
							{logo.kind === 'replace' && !saved.branding.has_logo ? 'Clear' : 'Remove logo'}
						</Button>
					{/if}
				</div>
				{#if m.problem}
					<p class={['mt-1 text-[11px]', statusText.warn]}>{m.problem}</p>
				{:else if logo.kind === 'remove'}
					<p class={['mt-1 text-[11px]', statusText.warn]}>The logo is removed when you save.</p>
				{:else if logo.kind === 'replace'}
					<p class="text-muted-foreground mt-1 text-[11px]">
						{kilobytes(logo.file.size)}, uploaded when you save.
					</p>
				{:else if saved.branding.has_logo && saved.branding.logo_type && saved.branding.logo_bytes !== null}
					<p class="text-muted-foreground mt-1 text-[11px]">
						{LOGO_TYPE_LABEL[saved.branding.logo_type]}, {kilobytes(saved.branding.logo_bytes)}
					</p>
				{/if}
			</SettingRow>
		</SettingsSection>

		<SettingsSection title="How your emails look">
			<div class="space-y-3 px-5 py-4">
				{#if emails}
					<EmailLookCard look={emails.current} label="Email preview" {logoUrl} />
					<p class="text-muted-foreground text-xs">
						{#if previewNote(emails) === 'no_name'}
							Set your operator name above; until then emails are sent as Marcan.
						{:else if previewNote(emails) === 'licensed'}
							Your operator license is active: your letterhead alone tops the emails, and they end with “{emails.current
								.sign_off}”
						{:else}
							Marcan opens every email and your letterhead comes second. With
							<SettingsLink to="license">an operator license</SettingsLink> your letterhead stands alone.
						{/if}
					</p>
					<p class="text-muted-foreground pt-2 text-xs font-medium">
						{emails.other.notice_shown ? 'Without an operator license' : 'With an operator license'}
					</p>
					<EmailLookCard
						look={emails.other}
						label={emails.other.notice_shown
							? 'Email preview without an operator license'
							: 'Email preview with an operator license'}
						{logoUrl}
					/>
				{:else}
					<Skeleton variant="block" class="h-32 max-w-xl" />
				{/if}
			</div>
		</SettingsSection>
	{/snippet}
</SettingsPart>
