import type { TimeseriesPoint } from '$lib/types/TimeseriesPoint';

export type DailyPoint = {
	dateObj: Date;
	messages: number;
	compliant: number;
	/** Quarantined or rejected -- policy working. */
	blocked: number;
	/** Failed DMARC, delivered because the receiver trusted the forwarding (ARC, lists). */
	forwarded: number;
	/** Delivered failures: DMARC-failing messages that were still delivered. */
	delivered: number;
};

/** Adapt `/api/timeseries` buckets to the shape the charts bind to. */
export function seriesFromTimeseries(points: TimeseriesPoint[]): DailyPoint[] {
	return points.map((p) => ({
		dateObj: new Date(p.ts * 1000),
		messages: p.total_messages,
		compliant: p.compliant_messages,
		blocked: p.blocked_messages,
		forwarded: p.forwarded_failures,
		delivered: p.delivered_failures
	}));
}
