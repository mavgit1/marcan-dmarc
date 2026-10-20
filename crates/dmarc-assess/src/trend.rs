//! Change against the previous window, and whether that change is good news.

use crate::status::Status;
use serde::Serialize;
use ts_rs::TS;

/// Which way a figure moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Up,
    Down,
    /// Less than 0.05 (the one decimal shown) either way.
    Flat,
    /// None in the previous window, some now: no percentage to show.
    New,
}

/// What a rise of the figure means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendSense {
    /// Compliance: up is good.
    HigherIsBetter,
    /// Delivered failures: up is bad.
    HigherIsWorse,
    /// Volume, blocked, forwarded: a change worth noticing, neither good nor bad.
    Neutral,
}

/// A figure against the previous window. `delta` is a relative change in
/// percent for counts and percentage points for rates; `null` when `new`.
/// `status`: `pass` / `fail` for good / bad news, `info` for a neutral
/// change, `neutral` when flat.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Trend {
    pub direction: TrendDirection,
    pub delta: Option<f64>,
    pub status: Status,
}

fn status_of(direction: TrendDirection, sense: TrendSense) -> Status {
    let up = match direction {
        TrendDirection::Flat => return Status::Neutral,
        TrendDirection::Up | TrendDirection::New => true,
        TrendDirection::Down => false,
    };
    match (sense, up) {
        (TrendSense::Neutral, _) => Status::Info,
        (TrendSense::HigherIsBetter, true) | (TrendSense::HigherIsWorse, false) => Status::Pass,
        _ => Status::Fail,
    }
}

/// A change of `delta` (percent or percentage points).
pub fn points_trend(delta: f64, sense: TrendSense) -> Trend {
    let direction = if (delta.abs() * 10.0).round() == 0.0 {
        TrendDirection::Flat
    } else if delta > 0.0 {
        TrendDirection::Up
    } else {
        TrendDirection::Down
    };
    Trend {
        direction,
        delta: Some(delta),
        status: status_of(direction, sense),
    }
}

/// The relative change of a count.
pub fn count_trend(current: i64, previous: i64, sense: TrendSense) -> Trend {
    if previous == 0 {
        if current == 0 {
            return points_trend(0.0, sense);
        }
        return Trend {
            direction: TrendDirection::New,
            delta: None,
            status: status_of(TrendDirection::New, sense),
        };
    }
    points_trend((current - previous) as f64 * 100.0 / previous as f64, sense)
}

/// Seconds in a day.
const DAY_SECS: i64 = 86_400;

/// The `[since, until)` window of equal length before a scope's `since`, when
/// the scope's reports cover it, for the "compared with the previous period"
/// figures. `first` and `last` are the `date_begin` of the scope's oldest and
/// newest report over all time.
///
/// The current window ends at the newest report, not at `now`: reports arrive
/// a day or two late, and comparing those empty days against a complete
/// previous window would read as a drop. The previous window only counts when
/// the reports reach back to its start, with a day of slack because daily
/// reports start at midnight and windows rarely do.
pub fn previous_window(
    since: Option<i64>,
    first: Option<i64>,
    last: Option<i64>,
    now: i64,
) -> Option<(i64, i64)> {
    let (since, first, last) = (since?, first?, last?);
    let horizon = last.saturating_add(DAY_SECS).min(now);
    let window = horizon.saturating_sub(since);
    let prev_since = since.saturating_sub(window);
    (window > 0 && covers_previous(Some(first), prev_since)).then_some((prev_since, since))
}

/// Whether reports beginning at `first` cover a previous period starting at
/// `prev_since` (a day of slack), so its figures compare like with like:
/// the rule [`previous_window`] uses, for a period chosen otherwise (the
/// calendar month before a client report's).
pub fn covers_previous(first: Option<i64>, prev_since: i64) -> bool {
    first.is_some_and(|first| first <= prev_since.saturating_add(DAY_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sense_decides_the_colour_not_direction() {
        let up = count_trend(12, 10, TrendSense::HigherIsWorse);
        assert_eq!(up.direction, TrendDirection::Up);
        assert_eq!(up.delta, Some(20.0));
        assert_eq!(up.status, Status::Fail);
        assert_eq!(
            count_trend(8, 10, TrendSense::HigherIsWorse).status,
            Status::Pass
        );
        assert_eq!(
            points_trend(1.5, TrendSense::HigherIsBetter).status,
            Status::Pass
        );
        assert_eq!(
            points_trend(-1.5, TrendSense::HigherIsBetter).status,
            Status::Fail
        );
        assert_eq!(count_trend(5, 10, TrendSense::Neutral).status, Status::Info);
    }

    #[test]
    fn flat_below_the_shown_decimal() {
        let flat = points_trend(0.04, TrendSense::HigherIsBetter);
        assert_eq!(
            (flat.direction, flat.status),
            (TrendDirection::Flat, Status::Neutral)
        );
        assert_eq!(
            points_trend(-0.06, TrendSense::Neutral).direction,
            TrendDirection::Down
        );
        assert_eq!(
            count_trend(0, 0, TrendSense::Neutral).direction,
            TrendDirection::Flat
        );
    }

    #[test]
    fn none_in_the_prior_window_is_new() {
        let new = count_trend(3, 0, TrendSense::HigherIsWorse);
        assert_eq!(new.direction, TrendDirection::New);
        assert_eq!(new.delta, None);
        assert_eq!(new.status, Status::Fail);
        assert_eq!(count_trend(3, 0, TrendSense::Neutral).status, Status::Info);
    }

    #[test]
    fn previous_window_needs_coverage() {
        let day = 86_400;
        // 30 days ending yesterday; reports reach back 90 days: covered.
        let now = 100 * day;
        let since = now - 30 * day;
        let win = previous_window(Some(since), Some(now - 90 * day), Some(now - day), now);
        assert_eq!(win, Some((since - 30 * day, since)));
        // Reports start inside the previous window: not comparable.
        assert_eq!(
            previous_window(Some(since), Some(since - 10 * day), Some(now - day), now),
            None
        );
        // A day of slack on the start.
        assert!(
            previous_window(Some(since), Some(since - 29 * day), Some(now - day), now).is_some()
        );
        assert_eq!(previous_window(None, Some(0), Some(now), now), None);
        assert_eq!(previous_window(Some(since), None, None, now), None);
    }
}
