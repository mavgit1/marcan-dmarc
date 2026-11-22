<script lang="ts">
	import { plural } from '#lib/format.ts';
	import type { AccountTestResponse } from '#lib/types/AccountTestResponse.ts';
	import type { LoginKind } from '#lib/types/LoginKind.ts';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import DocsSentence from './docs-sentence.svelte';

	/**
	 * What an account test answered, one line per step that ran: the token (a Microsoft login),
	 * the mailbox with its folders (over IMAP for a password login, over Microsoft Graph for a
	 * Microsoft one), the SMTP login. A failure says why, with a "(docs)" link when the server names
	 * the docs section on the fix (a Microsoft login's setup step), its raw error folded under Details.
	 */
	let { result, login }: { result: AccountTestResponse; login: LoginKind } = $props();

	/** How the mailbox step is named: by what it reads the mailbox over. */
	const MAILBOX_STEP: Record<LoginKind, string> = { password: 'IMAP', microsoft: 'Mailbox' };

	type Line = { name: string; ok: boolean; message: string; detail: string | null; docs: string | null };

	const lines = $derived.by((): Line[] => {
		const out: Line[] = [];
		if (result.token) out.push({ name: 'Token', ...result.token });
		if (result.mailbox) {
			const { ok, message, detail, docs, folders } = result.mailbox;
			const listed = ok ? ` ${plural(folders.length, 'folder')} listed.` : '';
			out.push({ name: MAILBOX_STEP[login], ok, message: `${message}${listed}`, detail, docs });
		}
		if (result.smtp) out.push({ name: 'SMTP', ...result.smtp });
		return out;
	});
</script>

<ul class="mt-3 max-w-md space-y-1.5 text-xs">
	{#each lines as line (line.name)}
		<li
			class={['flex min-w-0 items-start gap-1.5', line.ok ? 'text-status-pass-fg' : 'text-status-fail-fg']}
			role={line.ok ? undefined : 'alert'}
		>
			{#if line.ok}
				<CircleCheckIcon class="mt-px size-3.5 shrink-0" />
			{:else}
				<CircleAlertIcon class="mt-px size-3.5 shrink-0" />
			{/if}
			<span class="min-w-0 break-words">
				<span class="font-medium">{line.name}:</span>
				<DocsSentence message={line.message} docs={line.docs} />
				{#if line.detail}
					<details class="mt-1">
						<summary class="cursor-pointer text-[11px] opacity-80">Details</summary>
						<span class="mt-1 block text-[11px] break-all">{line.detail}</span>
					</details>
				{/if}
			</span>
		</li>
	{/each}
</ul>
