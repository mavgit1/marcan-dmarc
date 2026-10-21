<script lang="ts">
	import type { SettingsPageId } from '$lib/settings-nav';
	import type { Snippet } from 'svelte';
	import type { Feature } from './settings-form.svelte';
	import { PageForm, setPageForm } from './page-form.svelte';
	import SaveBar from './save-bar.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPageHeader from './settings-page-header.svelte';
	import SettingsSection from './settings-section.svelte';
	import SwitchInput from './switch-input.svelte';

	/**
	 * A Settings page: its header, then one form holding every section that saves (`children`;
	 * each joins the page form, `joinPageForm`) ending in the page's one save bar, then `after`,
	 * outside the form: actions that act at once with their own feedback.
	 * A feature page passes `feature`: its switch is the form's first row, saved by the save bar
	 * like any field; while it is off, every row below it is disabled (one `fieldset`, dimmed) and
	 * keeps its values. The save bar is never disabled with them. `below` is the line under the
	 * switch naming what the feature cannot work without. `ungated` is what works whatever the
	 * switch says (Client reports' preview): in the form after the disabled rows, never dimmed.
	 */
	let {
		page,
		hint,
		docs,
		tag,
		feature,
		below,
		children,
		ungated,
		after
	}: {
		page: SettingsPageId;
		/** One sentence: what the page configures. */
		hint: string;
		/** The docs page and anchor, such as `mailbox#sending`. */
		docs: string;
		/** A label in the warn tone after the title, such as "Experimental". */
		tag?: string;
		/** The feature's switch; `null` until the settings are loaded. */
		feature?: Feature | null;
		/** One line under the switch: a hard dependency that is not met. */
		below?: Snippet;
		children: Snippet;
		/** After the rows the switch disables, outside them: what works while the feature is off. */
		ungated?: Snippet;
		after?: Snippet;
	} = $props();

	const form = new PageForm();
	setPageForm(form);

	const off = $derived(!!feature && !feature.on);
</script>

<SettingsPageHeader {page} {hint} {docs} {tag} />

<form onsubmit={(e) => form.submit(e)} class="space-y-4">
	{#if feature}
		<SettingsSection>
			<SettingRow title={feature.title} hint={feature.hint} labelFor="{page}-enabled">
				<SwitchInput id="{page}-enabled" checked={feature.on} onchange={(on) => feature.set(on)} />
				{#if below}
					<div class="mt-2">{@render below()}</div>
				{/if}
			</SettingRow>
		</SettingsSection>
	{/if}
	<!-- One fieldset: while the feature is off, every control below its switch is disabled with it. -->
	<fieldset disabled={off} class={['min-w-0 space-y-4 transition-opacity', off && 'opacity-60']}>
		{@render children()}
	</fieldset>
	{@render ungated?.()}
	<SaveBar dirty={form.dirty} save={form.save} reason={form.reason} onDiscard={() => form.discard()} />
</form>

{#if after}
	<div class="mt-8 space-y-4">{@render after()}</div>
{/if}
