<script lang="ts">
	import { getUpcomingClientReport } from '$lib/api';
	import ClientReportPreview from '$lib/components/client-report-preview.svelte';
	import * as Card from '$lib/components/ui/card/index.js';
	import { formatDay, plural } from '$lib/format';
	import { meta } from '$lib/state/meta.svelte';
	import { resource } from '$lib/state/resource.svelte';

	/**
	 * The organization page's client report line: from `client_report_notice_days` (GET
	 * /api/meta) before the send, when the report goes out and to how many recipients, with the
	 * preview. Nothing while the reports or sending are off, for an organization not in the
	 * recipient list, or earlier. While the due reports are held (`held`), it says so instead. A
	 * failed load shows nothing: the line only informs.
	 */
	let { organizationId, organizationName }: { organizationId: number; organizationName: string } = $props();

	const upcoming = resource({
		key: () => String(organizationId),
		load: () => getUpcomingClientReport(organizationId)
	});
	$effect(() => {
		meta.load().catch(() => {});
	});

	const DAY_SECS = 86_400;
	const shown = $derived.by(() => {
		const report = upcoming.data;
		const days = meta.current?.client_report_notice_days;
		if (!report || days === undefined) return null;
		return Date.now() / 1000 >= report.next_send_at - days * DAY_SECS ? report : null;
	});
</script>

{#if shown}
	<Card.Root class="mb-6 py-3">
		<Card.Content class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-4">
			<p class="text-sm">
				{#if shown.held}
					The client report due on {formatDay(shown.next_send_at)} is held until the mailbox is read.
				{:else}
					Client report goes out on {formatDay(shown.next_send_at)} to {plural(shown.recipients, 'recipient')}.
				{/if}
			</p>
			<ClientReportPreview {organizationId} {organizationName} />
		</Card.Content>
	</Card.Root>
{/if}
