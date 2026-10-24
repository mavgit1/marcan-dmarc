//! The update alert: a newer Marcan release than the running one, raised
//! once per version ([`update_events`]). Unlike the other kinds it is about
//! the installation, not a domain.
//!
//! The version check stores the newest release the docs site named; the
//! alert evaluation compares it with the newest version it has seen before
//! ([`UpdateAlertState`], stored between evaluations like a domain's
//! [`AlertState`](super::AlertState)). The first evaluation is its baseline
//! and raises nothing, so the first look after upgrading to the version that
//! added the alert does not announce a release the header already names.

use super::events::{AlertEvent, AlertKind};
use crate::status::Status;
use serde::{Deserialize, Serialize};

/// `X.Y.Z` as a tuple that orders as versions do; `None` for anything else
/// (a prefix, a pre-release, a sign, spaces).
pub fn release_version(text: &str) -> Option<(u64, u64, u64)> {
    // Digits only: `u64::from_str` also takes a leading `+`.
    let num = |p: &str| p.parse().ok().filter(|_| !p.starts_with('+'));
    let mut parts = text.split('.').map(num);
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// Whether `candidate` is a later release than `running`; `false` when
/// either is no `X.Y.Z`.
pub fn is_newer_release(candidate: &str, running: &str) -> bool {
    match (release_version(candidate), release_version(running)) {
        (Some(c), Some(r)) => c > r,
        _ => false,
    }
}

/// What the update alert keeps between evaluations: the newest version it
/// has seen while the check was on, newer than the running one or not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAlertState {
    /// "0.3.2"; `None` before the check first answered.
    pub latest: Option<String>,
}

/// The release the version check last named: its version and the link to
/// its release notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeenRelease<'a> {
    pub version: &'a str,
    pub url: &'a str,
}

/// The update alert's next state and its event, if any, from the `previous`
/// state (`None`: the first evaluation, a baseline), whether the check is on,
/// the release it last named (`seen`) and the `running` version. A release
/// is alerted once: when it is newer than the running version and than
/// every version seen before. Nothing while the check is off (the state
/// stays as it was), and nothing for a version no newer than the running one
/// (it is remembered, so it never alerts later either).
pub fn update_events(
    previous: Option<&UpdateAlertState>,
    check_on: bool,
    seen: Option<SeenRelease<'_>>,
    running: &str,
) -> (UpdateAlertState, Vec<AlertEvent>) {
    let mut state = previous.cloned().unwrap_or_default();
    let Some(seen) = seen.filter(|_| check_on) else {
        return (state, Vec::new());
    };
    let Some(version) = release_version(seen.version) else {
        return (state, Vec::new());
    };
    if state.latest.as_deref().and_then(release_version) >= Some(version) {
        return (state, Vec::new());
    }
    state.latest = Some(seen.version.to_owned());
    let mut events = Vec::new();
    if previous.is_some() && is_newer_release(seen.version, running) {
        events.push(AlertEvent::new(
            AlertKind::UpdateAvailable {
                version: seen.version.to_owned(),
                url: seen.url.to_owned(),
            },
            Status::Info,
        ));
    }
    (state, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "https://marcan.kaitiaki.tech/releases";

    fn seen(version: &str) -> Option<SeenRelease<'_>> {
        Some(SeenRelease {
            version,
            url: NOTES,
        })
    }

    fn latest(version: &str) -> UpdateAlertState {
        UpdateAlertState {
            latest: Some(version.into()),
        }
    }

    #[test]
    fn versions_parse_strictly_and_order_as_releases() {
        assert!(release_version("0.10.0") > release_version("0.9.9"));
        assert!(release_version("1.0.0") > release_version("0.99.99"));
        assert_eq!(release_version("0.3.2"), Some((0, 3, 2)));
        for bad in "|0.2|0.2.1.0|v0.2.1|0.2.1-rc1|0..1| 0.2.1|a.b.c|+0.2.1".split('|') {
            assert_eq!(release_version(bad), None, "{bad:?}");
        }
        assert!(is_newer_release("0.3.2", "0.3.1"));
        assert!(!is_newer_release("0.3.1", "0.3.1") && !is_newer_release("0.3.0", "0.3.1"));
        assert!(!is_newer_release("latest", "0.3.1") && !is_newer_release("0.3.2", "dev"));
    }

    /// A newer release alerts once, with its release notes, in the info
    /// colour; a second day with the same answer raises nothing, a later
    /// release raises again.
    #[test]
    fn a_newer_release_alerts_once_per_version() {
        let start = latest("0.3.1");
        let (state, events) = update_events(Some(&start), true, seen("0.3.2"), "0.3.1");
        assert_eq!(state, latest("0.3.2"));
        assert_eq!(
            events,
            [AlertEvent::new(
                AlertKind::UpdateAvailable {
                    version: "0.3.2".into(),
                    url: NOTES.into(),
                },
                Status::Info
            )]
        );
        let (again, events) = update_events(Some(&state), true, seen("0.3.2"), "0.3.1");
        assert_eq!((again, events), (latest("0.3.2"), vec![]));
        let (next, events) = update_events(Some(&state), true, seen("0.4.0"), "0.3.1");
        assert_eq!(next, latest("0.4.0"));
        assert_eq!(events.len(), 1);
        // The site going back to an older answer is no news, and the newer
        // one is not alerted again when it returns.
        let (back, events) = update_events(Some(&next), true, seen("0.3.2"), "0.3.1");
        assert_eq!((back.clone(), events), (latest("0.4.0"), vec![]));
        assert!(update_events(Some(&back), true, seen("0.4.0"), "0.3.1")
            .1
            .is_empty());
    }

    /// The running version or an older one is never alerted, and it is
    /// remembered: running an older image later does not alert it either.
    #[test]
    fn the_running_version_or_an_older_one_raises_nothing() {
        let start = UpdateAlertState::default();
        for version in ["0.3.1", "0.3.0", "0.2.9"] {
            let (state, events) = update_events(Some(&start), true, seen(version), "0.3.1");
            assert_eq!((state, events), (latest(version), vec![]), "{version}");
        }
        let (state, _) = update_events(Some(&start), true, seen("0.3.1"), "0.3.1");
        assert!(update_events(Some(&state), true, seen("0.3.1"), "0.3.0")
            .1
            .is_empty());
        // An answer that is no version changes nothing.
        let (state, events) = update_events(Some(&start), true, seen("latest"), "0.3.1");
        assert_eq!((state, events), (UpdateAlertState::default(), vec![]));
    }

    /// While the check is off nothing is raised and the state stays; switched
    /// on again, a release newer than the last one seen alerts.
    #[test]
    fn nothing_while_the_check_is_off() {
        let start = latest("0.3.1");
        let (state, events) = update_events(Some(&start), false, seen("0.3.2"), "0.3.1");
        assert_eq!((state.clone(), events), (start.clone(), vec![]));
        assert!(update_events(Some(&start), true, None, "0.3.1")
            .1
            .is_empty());
        assert_eq!(
            update_events(Some(&state), true, seen("0.3.2"), "0.3.1")
                .1
                .len(),
            1
        );
    }

    /// The first evaluation records the release and raises nothing, newer or
    /// not; from there a later release alerts. A first answer that comes
    /// after the first evaluation alerts when it is newer.
    #[test]
    fn the_first_evaluation_is_a_baseline() {
        let (state, events) = update_events(None, true, seen("0.3.2"), "0.3.1");
        assert_eq!((state.clone(), events), (latest("0.3.2"), vec![]));
        assert!(update_events(Some(&state), true, seen("0.3.2"), "0.3.1")
            .1
            .is_empty());
        assert_eq!(
            update_events(Some(&state), true, seen("0.3.3"), "0.3.1")
                .1
                .len(),
            1
        );
        let (unanswered, _) = update_events(None, true, None, "0.3.1");
        assert_eq!(unanswered, UpdateAlertState::default());
        assert_eq!(
            update_events(Some(&unanswered), true, seen("0.3.2"), "0.3.1")
                .1
                .len(),
            1
        );
    }

    #[test]
    fn the_state_round_trips_as_json() {
        let state = latest("0.3.2");
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<UpdateAlertState>(&json).unwrap(),
            state
        );
    }
}
