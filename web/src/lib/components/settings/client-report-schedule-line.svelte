<script lang="ts">
	import { formatDay, plural } from '$lib/format';
	import { settings } from '$lib/state/settings.svelte';
	import { statusText } from '$lib/tokens';

	/**
	 * The send line under the Client reports switch, from the saved settings: when the reports
	 * go out next and to how many organizations, and when they last went out; under it, in the
	 * warn tone, why the due reports are held (`held_reason`), and the latest report time's
	 * reports that did not reach a recipient (`undelivered`: the server's line, in its colour).
	 * Nothing while they are off as saved.
	 */
	const reports = $derived(settings.current?.client_reports ?? null);
	const held = $derived(reports?.held_reason ?? null);
	const undelivered = $derived(reports?.undelivered ?? null);
</script>

{#if reports?.enabled && reports.next_send_at !== null}
	<p class="text-muted-foreground text-xs">
		Next send: {formatDay(reports.next_send_at)}, {plural(reports.organizations_due, 'organization')}.
		{reports.last_sent_at === null ? 'Not sent yet.' : `Last sent: ${formatDay(reports.last_sent_at)}.`}
	</p>
	{#if held?.kind === 'mailbox_not_read'}
		<p class={['text-xs', statusText.warn]}>
			{#if held.last_read_at === null}
				Held: the mailbox has not been read yet. Reports go out once it is read.
			{:else}
				Held: the mailbox has not been read since {formatDay(held.last_read_at)}. Reports go out once it is read again.
			{/if}
		</p>
	{/if}
	{#if undelivered}
		<p class={['text-xs', statusText[undelivered.status]]}>{undelivered.line}</p>
	{/if}
{/if}
