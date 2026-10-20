<script lang="ts">
	import { reimportMailbox } from '$lib/api';
	import { systemHealth } from '$lib/state/system-health.svelte';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import { Action } from '../action.svelte';
	import BusyButton from '../busy-button.svelte';

	/**
	 * Re-import the mailbox: reads every message again and adds the reports that are
	 * missing. Busy while any import runs; its progress is the header's status.
	 */
	let { size = 'sm' }: { size?: 'sm' | 'xs' } = $props();

	const run = new Action();
	const importing = $derived(systemHealth.current?.imap_progress != null);

	function onClick() {
		run.run(async () => {
			const res = await reimportMailbox();
			await systemHealth.poll();
			if (!res.started) return { ok: false, message: res.message };
		});
	}
</script>

<span class="inline-flex flex-wrap items-center gap-2">
	<BusyButton
		type="button"
		variant="outline"
		{size}
		busy={run.busy || importing}
		busyLabel="Importing…"
		onclick={onClick}
	>
		<RotateCcwIcon class="mr-1.5 size-3.5" />
		Re-import mailbox
	</BusyButton>
	{#if run.feedback && !run.feedback.ok}
		<span class="text-status-fail-fg text-xs" role="alert">{run.feedback.message}</span>
	{/if}
</span>
