<script lang="ts">
	import { scopeOptions } from '$lib/state/scope.svelte';
	import BusyButton from '../busy-button.svelte';
	import ClientReportPreview from '../client-report-preview.svelte';
	import type { ClientReportsModel } from './client-reports-draft.svelte';
	import FeedbackAlert from './feedback-alert.svelte';
	import SettingRow from './setting-row.svelte';
	import SettingsSection from './settings-section.svelte';
	import TestRecipientLine from './test-recipient-line.svelte';

	/**
	 * Client reports' Preview, per organization: the report shown in the app (`ClientReportPreview`,
	 * which works whatever the switch says: the server renders it regardless) and "Send a preview"
	 * to the test recipient, which waits for the switch and sending (`testBlocked`). Outside the
	 * rows the switch disables (`SettingsPage`'s `ungated`).
	 */
	let { model: m }: { model: ClientReportsModel } = $props();

	$effect(() => {
		scopeOptions.load().catch(() => {
			// The preview list stays empty; the rest of the page works.
		});
	});
	const organizations = $derived(scopeOptions.current?.organizations ?? []);
</script>

{#snippet note(text: string)}
	<p class="text-muted-foreground mt-1 text-[11px]">{text}</p>
{/snippet}

<!-- The form's own part shows its skeleton and load error; this follows once it loaded. -->
{#if m.saved}
	<SettingsSection title="Preview">
		<SettingRow title="Organizations" hint="Each report as it would go out today." stacked>
			<ul class="max-w-md divide-y rounded-md border">
				{#each organizations as org (org.id)}
					<li class="px-3 py-2">
						<div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
							<span class="min-w-0 truncate text-sm">{org.name}</span>
							<div class="flex shrink-0 items-center gap-2">
								<ClientReportPreview organizationId={org.id} organizationName={org.name} />
								<BusyButton
									type="button"
									variant="outline"
									size="sm"
									busy={m.test.busy && m.previewing === org.id}
									busyLabel="Sending…"
									disabled={!!m.testBlocked || m.test.busy}
									onclick={() => m.sendPreview(org.id)}
								>
									Send a preview
								</BusyButton>
							</div>
						</div>
						{#if m.test.feedback && m.previewing === org.id}
							<div class="mt-2">
								<FeedbackAlert feedback={m.test.feedback} failTitle="The preview was not sent" />
							</div>
						{/if}
					</li>
				{/each}
			</ul>
			{#if !m.draft.enabled}
				{@render note('Preview works before you turn client reports on.')}
			{/if}
			{#if m.testBlocked}
				{@render note(m.testBlocked)}
			{:else}
				<TestRecipientLine to={m.testRecipient} />
			{/if}
		</SettingRow>
	</SettingsSection>
{/if}
