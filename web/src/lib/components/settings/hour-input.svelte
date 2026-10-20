<script lang="ts">
	import { cn } from '$lib/utils.js';
	import { hourToTime, timeToHour } from './time-of-day';

	/**
	 * The hour of a schedule (0–23, UTC) as a compact time field showing "07:00". Settings store
	 * whole hours, so the minutes are set back to 00 once a time is entered; a cleared field
	 * keeps the hour it had. The label beside it says UTC. Never a 24-entry select.
	 */
	let {
		id,
		value = $bindable(),
		disabled = false,
		class: className
	}: { id: string; value: number; disabled?: boolean; class?: string } = $props();

	function commit(e: Event & { currentTarget: HTMLInputElement }) {
		const hour = timeToHour(e.currentTarget.value);
		if (hour !== null) value = hour;
		e.currentTarget.value = hourToTime(Number(value));
	}
</script>

<input
	{id}
	type="time"
	step="3600"
	value={hourToTime(Number(value))}
	{disabled}
	onchange={commit}
	onblur={commit}
	class={cn(
		'border-input bg-background h-9 w-28 min-w-0 rounded-md border px-2 text-sm tabular-nums shadow-xs disabled:opacity-50',
		className
	)}
/>
