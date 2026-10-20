export type ChartSeries = {
	key: string;
	label: string;
	/** CSS colour. Defaults to `var(--chart-N)` cycling 1–5. */
	color?: string;
};

export type ChartDatum = Record<string, string | number | Date | null | undefined>;

function seriesColor(series: ChartSeries, index: number): string {
	return series.color ?? `var(--chart-${(index % 5) + 1})`;
}

export function toChartConfig(series: ChartSeries[]) {
	return Object.fromEntries(
		series.map((s, i) => [
			s.key,
			{
				label: s.label,
				color: seriesColor(s, i)
			}
		])
	);
}

export function formatChartDate(d: unknown): string {
	return d instanceof Date
		? d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' })
		: String(d ?? '');
}
