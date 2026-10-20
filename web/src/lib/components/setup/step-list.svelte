<script lang="ts">
	import { cn } from '$lib/utils.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { SETUP_STEPS } from './setup-steps';

	/**
	 * The steps in their order: done (a check), the one on screen, and the ones to come. A step
	 * can be opened from here once it was reached (`reachable`); the titles show from `md` up,
	 * narrower only the one on screen, under the numbers.
	 */
	let {
		current,
		done,
		reachable,
		onSelect
	}: {
		current: number;
		done: readonly boolean[];
		reachable: (index: number) => boolean;
		onSelect: (index: number) => void;
	} = $props();
</script>

<nav aria-label="Setup steps" class="space-y-2">
	<ol class="grid grid-cols-7 gap-1">
		{#each SETUP_STEPS as step, i (step.id)}
			{@const isCurrent = i === current}
			{@const isDone = done[i]}
			<li class="min-w-0">
				<button
					type="button"
					class={cn(
						'group flex w-full flex-col items-center gap-1.5 rounded-md px-1 py-1.5 text-center transition-colors',
						reachable(i) && !isCurrent ? 'hover:bg-muted/50' : 'cursor-default'
					)}
					disabled={!reachable(i)}
					aria-current={isCurrent ? 'step' : undefined}
					aria-label={`${step.title}${isDone ? ', done' : ''}`}
					onclick={() => onSelect(i)}
				>
					<span
						class={cn(
							'flex size-7 shrink-0 items-center justify-center rounded-full text-xs font-semibold',
							isCurrent
								? 'bg-primary text-primary-foreground'
								: isDone
									? 'bg-status-pass-bg text-status-pass-fg'
									: 'bg-muted text-muted-foreground',
							isCurrent && isDone && 'ring-status-pass-border ring-2 ring-offset-1'
						)}
					>
						{#if isDone && !isCurrent}
							<CheckIcon class="size-3.5" aria-hidden="true" />
						{:else}
							{i + 1}
						{/if}
					</span>
					<span
						class={cn(
							'hidden text-[11px] leading-tight md:block',
							isCurrent ? 'text-foreground font-medium' : 'text-muted-foreground'
						)}
					>
						{step.title}
					</span>
				</button>
			</li>
		{/each}
	</ol>
	<p class="text-muted-foreground text-center text-xs md:hidden" aria-hidden="true">
		Step {current + 1} of {SETUP_STEPS.length}: {SETUP_STEPS[current].title}
	</p>
</nav>
