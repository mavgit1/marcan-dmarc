<script lang="ts">
	import { formatDay, plural } from '$lib/format';
	import { settings } from '$lib/state/settings.svelte';
	import { statusText } from '$lib/tokens';

	/**
	 * The send line under the Client reports switch, from the saved settings: when the reports
	 * go out next and to how many organizations, and when they last went out; under it, in the
	 * warn tone, why the due reports are held (`held_reason`: the account unread longest), and the latest report time's
	 * reports that did not reach a recipient (`undelivered`: the server's line, in its colour).
	 * Nothing while they are off as saved; no send line while sending is (`SendingOffLine` says
	 * so instead: nothing goes out then).
	 */
	const reports = $derived(settings.current?.client_reports ?? null);
	const sending = $derived(settings.current?.sending.enabled ?? false);
	const held = $derived(reports?.held_reason ?? null);
	const undelivered = $derived(reports?.undelivered ?? null);
</script>

{#if reports?.enabled && reports.next_send_at !== null}
	{#if sending}
		<p class="text-muted-foreground text-xs">
			Next send: {formatDay(reports.next_send_at)}, {plural(reports.organizations_due, 'organization')}.
			{reports.last_sent_at === null ? 'Not sent yet.' : `Last sent: ${formatDay(reports.last_sent_at)}.`}
		</p>
	{/if}
	{#if held?.kind === 'mailbox_not_read'}
		<p class={['text-xs', statusText.warn]}>
			{#if held.account === null}
				Held: no account receives reports. Reports go out once one is read.
			{:else if held.last_read_at === null}
				Held: the account <span class="font-medium">{held.account}</span> has not been read since it was set up on {formatDay(
					held.receiving_since ?? 0
				)}. Reports go out once it is read.
			{:else}
				Held: the account <span class="font-medium">{held.account}</span> has not been read since {formatDay(
					held.last_read_at
				)}. Reports go out once it is read again.
			{/if}
		</p>
	{/if}
	{#if undelivered}
		<p class={['text-xs', statusText[undelivered.status]]}>{undelivered.line}</p>
	{/if}
{/if}
