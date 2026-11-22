<script lang="ts">
	import { joinPageForm } from '#lib/components/settings/page-form.svelte.ts';
	import { typed, type ResolversModel } from '#lib/components/settings/resolvers-draft.svelte.ts';
	import SettingRow from '#lib/components/settings/setting-row.svelte';
	import SettingsPart from '#lib/components/settings/settings-part.svelte';
	import { Input } from '#lib/components/ui/input/index.ts';

	/**
	 * The DNS checks page's resolvers (`model`, the dns group), saved by the page's save bar. While
	 * both fields are empty, their placeholders name the defaults used instead.
	 */
	let { model: m }: { model: ResolversModel } = $props();
	joinPageForm(() => m);
</script>

<SettingRow title="Resolvers" hint="Clear both to use the defaults." anchor="dns-resolvers">
	<SettingsPart form={m} errorTitle="DNS settings failed to load" skeleton="h-16">
		{#snippet children(saved)}
			{@const [primary, secondary] = typed(m.draft).length === 0 ? saved.dns.defaults : []}
			<div class="grid gap-3 @md:grid-cols-2">
				<label class="space-y-1">
					<span class="text-muted-foreground text-xs font-medium">Primary</span>
					<Input
						bind:value={m.draft.primary}
						placeholder={primary ? `Default: ${primary}` : 'Not set'}
						class="font-mono text-sm"
					/>
				</label>
				<label class="space-y-1">
					<span class="text-muted-foreground text-xs font-medium">Secondary</span>
					<Input
						bind:value={m.draft.secondary}
						placeholder={secondary ? `Default: ${secondary}` : 'Not set'}
						class="font-mono text-sm"
					/>
				</label>
			</div>
		{/snippet}
	</SettingsPart>
</SettingRow>
