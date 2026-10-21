<script lang="ts">
	import { previewBranding } from '$lib/api';
	import EmailLookCard from '$lib/components/settings/email-look-card.svelte';
	import SettingsLink from '$lib/components/settings/settings-link.svelte';
	import Skeleton from '$lib/components/skeleton.svelte';
	import { storedLogo } from '$lib/state/logo.svelte';
	import { settings } from '$lib/state/settings.svelte';
	import type { EmailPreview } from '$lib/types/EmailPreview';

	/**
	 * "What your clients see" on the License page, where an operator decides: the same email
	 * without and with an operator license, as the server renders them for the stored branding
	 * (`previewBranding`, the Branding page's preview; the server words the letterhead and the
	 * last line). Without a stored operator name it previews an example name, and says so.
	 */
	const EXAMPLE_NAME = 'Acme IT';

	const branding = $derived(settings.current?.branding ?? null);
	const name = $derived(branding?.operator_name.trim() || EXAMPLE_NAME);
	const example = $derived(!!branding && !branding.operator_name.trim());

	let preview = $state.raw<EmailPreview | null>(null);
	let seq = 0;
	$effect(() => {
		if (!branding) return;
		// The license state (`current` and `other`) follows a key save through the shared settings.
		void settings.current?.license.state;
		const request = { operator_name: name, has_logo: branding.has_logo };
		const id = ++seq;
		previewBranding(request)
			.then((p) => {
				if (id === seq) preview = p;
			})
			.catch(() => {
				// No preview: the section keeps its skeleton; the Branding page shows the same.
			});
	});
	const logoUrl = $derived(storedLogo.url(branding?.has_logo ?? false));
	const without = $derived(preview ? (preview.current.notice_shown ? preview.current : preview.other) : null);
	const licensed = $derived(preview ? (preview.current.notice_shown ? preview.other : preview.current) : null);
</script>

<section aria-labelledby="license-client-emails" class="space-y-3">
	<h2 id="license-client-emails" class="text-sm font-medium">What your clients see</h2>
	{#if without && licensed}
		<div class="grid gap-4 xl:grid-cols-2">
			<div class="min-w-0 space-y-1.5">
				<p class="text-muted-foreground text-xs font-medium">Without an operator license</p>
				<EmailLookCard look={without} label="Client email without an operator license" {logoUrl} />
			</div>
			<div class="min-w-0 space-y-1.5">
				<p class="text-muted-foreground text-xs font-medium">With an operator license</p>
				<EmailLookCard look={licensed} label="Client email with an operator license" {logoUrl} />
			</div>
		</div>
		<p class="text-muted-foreground text-xs">
			{#if example}
				Shown with the example name {EXAMPLE_NAME}; set yours under <SettingsLink to="branding" />.
			{:else}
				Your name and logo come from <SettingsLink to="branding" />.
			{/if}
		</p>
	{:else}
		<div class="grid gap-4 xl:grid-cols-2">
			<Skeleton variant="block" class="h-40" />
			<Skeleton variant="block" class="h-40" />
		</div>
	{/if}
</section>
