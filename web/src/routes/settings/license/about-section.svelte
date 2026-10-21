<script lang="ts">
	import { joinPageForm } from '$lib/components/settings/page-form.svelte';
	import SettingsPart from '$lib/components/settings/settings-part.svelte';
	import SwitchInput from '$lib/components/settings/switch-input.svelte';
	import type { UpdatesModel } from '$lib/components/settings/updates-draft.svelte';
	import { DOCS_URL } from '$lib/nav';
	import { meta } from '$lib/state/meta.svelte';

	/**
	 * The foot of the License page, neutral text from `meta`: the version, the license (free
	 * software, no warranty, AGPLv3 plus one duty) and the source code. Not the page's headline:
	 * that is the license state box. Then the version check (`model`, the updates group), saved by
	 * the page's save bar: its switch, what it sends, and the newer version the server found
	 * (`meta`, as the sidebar shows it).
	 */
	let { model: m }: { model: UpdatesModel } = $props();
	joinPageForm(() => m);
	const version = $derived(meta.current?.version ?? null);
	/** The server's one source URL (also in the notice and the emails). */
	const sourceUrl = $derived(meta.current?.source_url ?? null);
	const available = $derived(meta.current?.available ?? null);
	$effect(() => {
		meta.load().catch(() => {});
	});
	const link = 'text-foreground underline underline-offset-2';
</script>

<section aria-labelledby="license-about" class="max-w-prose space-y-2">
	<h2 id="license-about" class="text-sm font-medium">About</h2>
	<p class="text-muted-foreground text-sm">
		Marcan <span class="tabular-nums">{version ?? '—'}</span>. Free software, no warranty: Beneficiary Copyleft License
		1.0, which is AGPLv3 plus one duty. Anyone you let use it may ask for the source code. The client emails tell your
		clients that it is free software (the free-software notice); an operator license replaces that notice with your
		name.
	</p>
	<p class="text-muted-foreground text-sm">
		Source code:
		{#if sourceUrl}
			<a href={sourceUrl} target="_blank" rel="noopener" class="{link} break-all">{sourceUrl}</a>
		{:else}
			—
		{/if}
		· <a href="{DOCS_URL}license" target="_blank" rel="noopener" class={link}>License</a>
	</p>
	<SettingsPart form={m} errorTitle="The version check failed to load" skeleton="h-10">
		<div class="flex items-start gap-3 pt-1">
			<SwitchInput id="license-update-check" checked={m.draft.check} onchange={(on) => (m.draft.check = on)} />
			<div class="min-w-0 space-y-0.5">
				<label for="license-update-check" class="cursor-pointer text-sm">Check for new versions daily</label>
				<p class="text-muted-foreground text-xs">
					Once a day Marcan fetches a small version file from marcan.kaitiaki.tech, which is hosted on GitHub Pages.
					Nothing about this installation is sent.
				</p>
			</div>
		</div>
		{#if available}
			<p class="text-sm">
				<a href={available.url} target="_blank" rel="noopener" class={link}>
					Marcan <span class="tabular-nums">{available.version}</span> is available
				</a>
			</p>
		{/if}
	</SettingsPart>
</section>
