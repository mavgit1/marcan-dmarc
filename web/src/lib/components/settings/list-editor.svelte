<script lang="ts" generics="S extends ListSummary">
	import { plural } from '$lib/format';
	import type { Snippet } from 'svelte';
	import type { ListDraft, ListSummary } from './list-draft.svelte';
	import { joinPageForm } from './page-form.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsPart from './settings-part.svelte';
	import SettingsSection from './settings-section.svelte';

	/**
	 * A list edited as plain text (`ListDraft`: the domains, the sender names, the extra DKIM
	 * selectors, the client report recipients), so a whole list can be pasted, reviewed or
	 * rewritten at once; it joins the page form. A plain textarea: what is wrong shows below it,
	 * "Fix 2 lines before saving:" and each line's error linked to its line, never as colours
	 * inside it; then what saving would change. A main list (`example` and `format`, the syntax)
	 * is a section of its own, as tall as its lines (at least 12 rows, at most 70vh, then it
	 * scrolls), with the Format panel always open: beside the editor from `lg` on, above it when
	 * narrower, never below a tall editor. A short token list (`row`: the DKIM selectors) is a
	 * setting row with a small box and no Format panel.
	 */
	let {
		list,
		title,
		hint,
		anchor,
		row = false,
		invalid = null,
		example,
		format
	}: {
		list: ListDraft<S>;
		/** The section's title (left out where the page's title says it), or the row's. */
		title?: string;
		/** The row's hint (`row` only). */
		hint?: string;
		/** The section's id a link opens (`SETTINGS_SECTIONS`). */
		anchor?: string;
		/** A setting row with a small box, for a short list. */
		row?: boolean;
		/**
		 * Why another part of the page holds Save because of this list ("Add at least one recipient
		 * to the list first."): the list is marked invalid and the reason stands under it.
		 */
		invalid?: string | null;
		/** A few lines of the list, the top of the Format panel. */
		example?: string;
		/** The syntax, as list items under the example. */
		format?: Snippet;
	} = $props();

	joinPageForm(() => list);

	const name = $derived(list.spec.name ?? list.spec.label);
	const errors = $derived(list.errors);
	const changes = $derived(list.dirty ? list.changes : null);
	/** Lines with at least one error. */
	const badLines = $derived(new Set(errors.map((e) => e.line)).size);
	const lineCount = $derived(list.draft.text.split('\n').length + 1);
	/** One row per line and one to type into: a main list at least 12 (the height stops at 70vh), a short one 3 to 12. */
	const rows = $derived(row ? Math.min(12, Math.max(3, lineCount)) : Math.max(12, lineCount));

	let textarea = $state<HTMLTextAreaElement>();

	/** Select line `line` (1-based) in the editor. */
	function jump(line: number) {
		if (!textarea) return;
		const lines = list.draft.text.split('\n');
		const start = lines.slice(0, line - 1).reduce((n, l) => n + l.length + 1, 0);
		textarea.focus();
		textarea.setSelectionRange(start, start + (lines[line - 1]?.length ?? 0));
	}
</script>

{#snippet editor()}
	<div class={['grid gap-x-8 gap-y-3', format && 'lg:grid-cols-[minmax(0,1fr)_17rem]']}>
		{#if format}
			<div class="text-muted-foreground min-w-0 text-xs leading-relaxed lg:col-start-2 lg:row-start-1">
				<p class="text-foreground font-medium">Format</p>
				<div class="mt-2 space-y-3">
					{#if example}
						<pre
							class="bg-muted text-foreground overflow-x-auto rounded-md px-3 py-2 font-mono text-[11px] leading-relaxed">{example}</pre>
					{/if}
					<ul class="list-disc space-y-1 pl-4">{@render format()}</ul>
				</div>
			</div>
		{/if}
		<div class={['min-w-0 space-y-3 lg:col-start-1 lg:row-start-1', row && 'max-w-md']}>
			<textarea
				bind:this={textarea}
				bind:value={list.draft.text}
				aria-label={name}
				aria-invalid={errors.length > 0 || !!invalid || undefined}
				spellcheck="false"
				autocomplete="off"
				disabled={list.save.busy}
				placeholder={list.spec.placeholder}
				{rows}
				class={[
					'border-input focus-visible:border-ring focus-visible:ring-ring/50 placeholder:text-muted-foreground dark:bg-input/30 block w-full resize-y rounded-lg border bg-transparent px-3 py-2 font-mono leading-relaxed outline-none focus-visible:ring-3 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40',
					row ? 'text-sm' : 'max-h-[70vh] text-xs'
				]}></textarea>
			{#if invalid && errors.length === 0}
				<p class="text-status-warn-fg text-[11px]">{invalid}</p>
			{/if}
			{#if errors.length > 0}
				<ul
					class="border-status-fail-border bg-status-fail-bg text-status-fail-fg space-y-0.5 rounded-md border p-3 text-xs"
					role="alert"
				>
					<li class="font-medium">Fix {plural(badLines, 'line')} before saving:</li>
					{#each errors as e (`${e.line}:${e.message}`)}
						<li>
							<button type="button" class="font-medium underline-offset-2 hover:underline" onclick={() => jump(e.line)}>
								Line {e.line}
							</button>: {e.message}
						</li>
					{/each}
				</ul>
			{/if}
			{#if changes}
				<div class="space-y-2 rounded-md border p-3 text-xs">
					<p class="font-medium">{changes.summary ? `Saving will change: ${changes.summary}` : 'No changes'}</p>
					{#each changes.details as group, i (i)}
						{#if group.items.length > 0}
							{#if group.label}
								<details class="group">
									<summary class="text-muted-foreground hover:text-foreground cursor-pointer list-none select-none">
										<span class="group-open:hidden">▸</span><span class="hidden group-open:inline">▾</span>
										{group.label} ({group.items.length})
									</summary>
									<ul class="mt-1 space-y-0.5 pl-4 font-mono">
										{#each group.items as item (item)}
											<li class="break-all">{item}</li>
										{/each}
									</ul>
								</details>
							{:else}
								<ul class="space-y-0.5">
									{#each group.items as item (item)}
										<li class="break-all">{item}</li>
									{/each}
								</ul>
							{/if}
						{/if}
					{/each}
				</div>
			{/if}
		</div>
	</div>
{/snippet}

{#if row}
	<SettingRow title={title ?? name} {hint} {anchor}>
		<SettingsPart form={list} errorTitle="{name} failed to load" skeleton="h-20 max-w-md">
			{@render editor()}
		</SettingsPart>
	</SettingRow>
{:else}
	<SettingsPart form={list} errorTitle="{name} failed to load">
		<SettingsSection {title} {anchor}>
			<div class="px-5 py-4">{@render editor()}</div>
		</SettingsSection>
	</SettingsPart>
{/if}
