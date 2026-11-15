<script lang="ts">
	import { resolve } from '$app/paths';
	import { plural, relativeTime } from '$lib/format';
	import { MAILBOX_LABEL } from '$lib/pipeline';
	import { FORWARDED_NAMED } from '$lib/report-address';
	import { joinAnd } from '$lib/text';
	import { statusBorderL, statusDot, statusText, TONE_RANK, type StatusTone } from '$lib/tokens';
	import type { AccountHealth } from '$lib/types/AccountHealth';
	import type { AccountView } from '$lib/types/AccountView';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';

	/**
	 * One account on the Accounts page, a link to its page: its name and address (once, when the
	 * name is the address), where its reports come from once it is a report address (the server's
	 * `report_address`: its address, and the addresses forwarded to it, named up to
	 * `FORWARDED_NAMED`, else counted), "Sends mail" when Sending goes through it, its last check from
	 * the health answer, the server's sentence while its credentials cannot be used as they are
	 * (locked, expiring, expired), and a warning while
	 * another account reads the same mailbox and folder. The border takes the worst of the check,
	 * the credential and that warning (one verdict per card).
	 */
	let {
		account: a,
		health,
		sends
	}: {
		account: AccountView;
		/** Its health; `undefined` while it does not receive (or before the first answer). */
		health: AccountHealth | undefined;
		/** Sending goes through it (switched on). */
		sends: boolean;
	} = $props();

	/** What the server says about credentials that cannot be used as they are; `null` while they can. */
	const credential = $derived(a.credential.lock_text ?? a.credential.expiry_text);
	const forwarded = $derived(a.receiving.forwarded_from);
	/** The address shown beside the name: not when the name is the address. */
	const address = $derived(a.address === a.name ? null : a.address || '—');
	const tone = $derived.by((): StatusTone => {
		const tones: StatusTone[] = [health && a.receiving.enabled ? health.status : 'neutral'];
		if (credential) tones.push(a.credential.status);
		if (a.shared_mailbox) tones.push('warn');
		return tones.reduce((worst, t) => (TONE_RANK[t] > TONE_RANK[worst] ? t : worst), 'neutral');
	});
	const state = $derived(
		!a.receiving.enabled
			? null
			: health
				? `${MAILBOX_LABEL[health.state]}${health.last_check_at !== null ? `, checked ${relativeTime(health.last_check_at)}` : ''}`
				: a.ready_to_receive
					? MAILBOX_LABEL.awaiting_first_poll
					: MAILBOX_LABEL.not_configured
	);
</script>

<a
	href={resolve(`/settings/accounts/${a.id}`)}
	class={[
		'bg-card ring-foreground/10 hover:bg-muted/40 flex min-w-0 items-start gap-3 rounded-xl border-l-4 px-4 py-3 ring-1 transition-colors',
		statusBorderL[tone]
	]}
>
	<div class="min-w-0 flex-1 space-y-1">
		<div class="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
			<h3 class="min-w-0 text-sm font-semibold break-words">{a.name}</h3>
			{#if address}
				<span class="text-muted-foreground min-w-0 text-xs break-all">{address}</span>
			{/if}
		</div>
		<ul class="text-muted-foreground space-y-0.5 text-xs">
			{#if a.report_address}
				<li class="break-words">Reports sent to <span class="text-foreground">{a.address}</span> land here</li>
			{/if}
			{#if a.report_address && forwarded.length > 0}
				<li class="break-words">
					Also reports forwarded from {forwarded.length > FORWARDED_NAMED
						? plural(forwarded.length, 'other address', 'other addresses')
						: joinAnd(forwarded)}
				</li>
			{/if}
			{#if sends}
				<li>Sends mail</li>
			{/if}
			{#if state}
				<li class="flex items-center gap-1.5">
					<span class={['size-1.5 shrink-0 rounded-full', statusDot[health?.status ?? 'neutral']]}></span>
					{state}
				</li>
			{/if}
			{#if credential}
				<li class={['break-words', statusText[a.credential.status]]}>{credential}</li>
			{/if}
			{#if a.shared_mailbox}
				<li class={['break-words', statusText.warn]}>{a.shared_mailbox}</li>
			{/if}
		</ul>
	</div>
	<ChevronRightIcon class="text-muted-foreground mt-0.5 size-4 shrink-0" />
</a>
