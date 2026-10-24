//! The alert history of the last weeks, read off the generated traffic the
//! way the server's daily evaluation would have seen it, judged by
//! `dmarc_assess` (delivered compliance, health, readiness). Only events are
//! stored, no `alert_state`: the server's first evaluation is then a
//! baseline and raises nothing of its own.

use crate::profile::{DnsClass, DAYS};
use crate::rng::Rng;
use crate::services::Services;
use crate::traffic::{Plan, Role, Traffic, DAY};
use dmarc_assess::{
    AlertEvent, AlertKind, AlertedIssue, CheckKind, DeliveredFigures, DmarcRecordFacts,
    EnforcementState, HealthState, Mail, NewSender, PolicyStep, ReadinessEvidence, RecordDelta,
    Status, StoppedFigures, NEW_SENDER_MIN_FAILURES, RATE_PASS_MIN_PCT, RATE_WARN_MIN_PCT,
    READINESS_MIN_MESSAGES, READINESS_WINDOW_DAYS,
};
use std::collections::{BTreeMap, BTreeSet};

/// How far back the history goes.
const HISTORY_DAYS: i64 = 42;
/// Evaluations a new state must hold before it counts (no flapping).
const HOLD: usize = 3;
/// Delivered mail in the window below which a domain's issue and health
/// are not re-judged: one spoofed message on a quiet domain is no story.
const MIN_DELIVERED: i64 = 40;
/// New failing senders kept per domain: a spoofing wave from many addresses
/// would otherwise fill the list.
const NEW_SENDERS_SHOWN: usize = 3;
/// Days without reports before they count as stopped (the default setting).
const STOPPED_AFTER_DAYS: i64 = 14;

/// One stored event: `(domain index, created_at, event)`.
pub type Stored = (usize, i64, AlertEvent);

fn window(t: &Traffic, last: usize) -> Mail {
    let first = (last + 1).saturating_sub(READINESS_WINDOW_DAYS as usize);
    let mut m = Mail::default();
    for day in &t.daily[first..=last] {
        let [passed, blocked, forwarded, delivered] = *day;
        m.total += passed + blocked + forwarded + delivered;
        m.compliant += passed;
        m.delivered_failures += delivered;
        m.forwarded_failures += forwarded;
    }
    m
}

fn figures(m: &Mail, status: Status) -> DeliveredFigures {
    let threshold = if status == Status::Fail {
        RATE_WARN_MIN_PCT
    } else {
        RATE_PASS_MIN_PCT
    };
    let delivered = m.compliant + m.delivered_failures;
    DeliveredFigures {
        window_days: READINESS_WINDOW_DAYS,
        delivered,
        delivered_failures: m.delivered_failures,
        blocked: m.blocked(),
        failure_pct: (delivered > 0)
            .then(|| m.delivered_failures as f64 * 100.0 / delivered as f64),
        threshold_pct: 100.0 - threshold,
    }
}

/// The delivered-failures issue's colour: `Pass` when there is none.
fn issue_status(m: &Mail) -> Status {
    let rate = m.delivered_compliance();
    if m.delivered_failures > 0 && matches!(rate.status, Status::Fail | Status::Warn) {
        rate.status
    } else {
        Status::Pass
    }
}

fn record_facts(p: &str) -> DmarcRecordFacts {
    DmarcRecordFacts {
        p: Some(p.to_owned()),
        sp: None,
        pct: 100,
        rua: vec![format!("mailto:{}", crate::catalogue::REPORT_ADDRESS)],
        adkim: "r".into(),
        aspf: "r".into(),
        testing: false,
    }
}

/// A value that changes only once a new one has held for [`HOLD`] evaluations.
struct Held<T: PartialEq + Copy> {
    value: T,
    candidate: T,
    seen: usize,
}

impl<T: PartialEq + Copy> Held<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            candidate: value,
            seen: 0,
        }
    }

    /// The previous value when `next` has now held long enough.
    fn update(&mut self, next: T) -> Option<T> {
        if next == self.value {
            self.seen = 0;
            return None;
        }
        if next == self.candidate {
            self.seen += 1;
        } else {
            self.candidate = next;
            self.seen = 1;
        }
        if self.seen >= HOLD {
            let before = self.value;
            self.value = next;
            self.seen = 0;
            return Some(before);
        }
        None
    }
}

pub fn history(
    rng: &mut Rng,
    services: &Services,
    plans: &[Plan],
    traffic: &[Traffic],
    today: i64,
    now: i64,
) -> Vec<Stored> {
    let mut out: Vec<Stored> = Vec::new();
    let first_eval = (DAYS - HISTORY_DAYS) as usize;
    // An evaluation after day `e`'s reports: the next morning.
    let at = |rng: &mut Rng, e: usize| {
        let morning = today - (DAYS - e as i64 - 1) * DAY + 5 * 3600;
        (morning + rng.below(5400) as i64).min(now - 600)
    };
    let esp_names: BTreeMap<String, &str> = services
        .esps
        .iter()
        .flat_map(|e| e.hosts.iter().map(move |h| (h.ip.clone(), e.name)))
        .collect();

    for (d, (plan, t)) in plans.iter().zip(traffic).enumerate() {
        if !matches!(plan.role, Role::Shaped | Role::Clean) {
            if plan.role == Role::Parked {
                let e = first_eval + 6 + rng.below(20);
                out.push((
                    d,
                    at(rng, e),
                    AlertEvent {
                        kind: AlertKind::ParkedChanged { parked: true },
                        severity: Status::Neutral,
                    },
                ));
            }
            continue;
        }
        // The first day with reports: evaluations before it see nothing.
        let Some(start) = t.daily.iter().position(|day| day.iter().sum::<i64>() > 0) else {
            continue;
        };

        let mut issue: Option<Held<Status>> = None;
        let mut health: Option<Held<HealthState>> = None;
        let mut ready: Option<Held<bool>> = None;
        let mut known: BTreeSet<&str> = BTreeSet::new();
        let mut announced: BTreeSet<&str> = BTreeSet::new();
        for e in first_eval.max(start)..DAYS as usize {
            let m = window(t, e);
            if m.total == 0 {
                continue;
            }
            let created = at(rng, e);
            // Too little delivered mail to say much: hold the last state.
            let quiet = m.compliant + m.delivered_failures < MIN_DELIVERED;
            let status = match (&issue, quiet) {
                (Some(held), true) => held.value,
                _ => issue_status(&m),
            };
            let state = match (&health, quiet) {
                (Some(held), true) => held.value,
                _ => m.health().state,
            };
            let is_ready = plan.policy_on(DAYS - e as i64) == "none"
                && e >= start + READINESS_WINDOW_DAYS as usize
                && m.total >= READINESS_MIN_MESSAGES
                && m.delivered_compliance()
                    .pct
                    .is_some_and(|p| p >= RATE_PASS_MIN_PCT);
            let first = issue.is_none();
            let issue = issue.get_or_insert_with(|| Held::new(status));
            let health = health.get_or_insert_with(|| Held::new(state));
            let ready = ready.get_or_insert_with(|| Held::new(is_ready));

            // New failing senders: nothing as the domain before the window.
            let since = (e + 1).saturating_sub(READINESS_WINDOW_DAYS as usize);
            let mut now_new: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
            let mut before: BTreeSet<&str> = BTreeSet::new();
            for (day, ip, delivered, blocked) in &t.failing {
                if *day < since {
                    before.insert(ip);
                } else if *day <= e {
                    let entry = now_new.entry(ip).or_default();
                    entry.0 += delivered + blocked;
                    entry.1 += delivered;
                }
            }
            now_new.retain(|ip, (failed, _)| {
                !before.contains(ip) && *failed >= NEW_SENDER_MIN_FAILURES
            });
            if !first {
                for (ip, (failed, delivered)) in &now_new {
                    if known.contains(ip)
                        || announced.contains(ip)
                        || announced.len() >= NEW_SENDERS_SHOWN
                    {
                        continue;
                    }
                    announced.insert(ip);
                    let severity = if *delivered > 0 {
                        Status::Fail
                    } else {
                        Status::Warn
                    };
                    out.push((
                        d,
                        created,
                        AlertEvent {
                            kind: AlertKind::NewFailingSender {
                                sender: NewSender {
                                    source_ip: (*ip).to_owned(),
                                    name: esp_names.get(*ip).map(|n| (*n).to_owned()),
                                    failed: *failed,
                                    delivered_failures: *delivered,
                                },
                                window_days: READINESS_WINDOW_DAYS,
                            },
                            severity,
                        },
                    ));
                }
            }
            known = now_new.keys().copied().collect();
            if first {
                continue;
            }

            if let Some(was) = issue.update(status) {
                let opened = was == Status::Pass;
                if opened || status == Status::Pass {
                    let kind = if opened {
                        AlertKind::IssueOpened {
                            issue: AlertedIssue::DeliveredFailures,
                            mail: Some(figures(&m, status)),
                            stopped: None,
                        }
                    } else {
                        AlertKind::IssueResolved {
                            issue: AlertedIssue::DeliveredFailures,
                            mail: Some(figures(&window(t, e - 1), Status::Pass)),
                            stopped: None,
                        }
                    };
                    let severity = if opened { status } else { Status::Pass };
                    out.push((d, created, AlertEvent { kind, severity }));
                }
            }
            if let Some(was) = health.update(state) {
                if was != HealthState::NoData && state != HealthState::NoData {
                    out.push((
                        d,
                        created,
                        AlertEvent {
                            kind: AlertKind::HealthChanged {
                                from: was,
                                to: state,
                                mail: Some(figures(&m, state.status())),
                            },
                            severity: state.status(),
                        },
                    ));
                }
            }
            if ready.update(is_ready) == Some(false) {
                out.push((
                    d,
                    created,
                    AlertEvent {
                        kind: AlertKind::ReadinessReached {
                            next: PolicyStep::Quarantine,
                            evidence: Some(ReadinessEvidence {
                                policy: EnforcementState::Monitor,
                                days: READINESS_WINDOW_DAYS,
                                messages: m.total,
                                delivered_compliance_pct: m.delivered_compliance().pct,
                            }),
                        },
                        severity: Status::Pass,
                    },
                ));
            }
        }

        // A tightened policy.
        if let Some((changed, before)) = plan.policy_before {
            if changed < HISTORY_DAYS {
                let e = (DAYS - changed) as usize;
                out.push((
                    d,
                    at(rng, e.min(DAYS as usize - 1)),
                    AlertEvent {
                        kind: AlertKind::PolicyChanged {
                            from: record_facts(before),
                            to: record_facts(plan.shape.dns.policy()),
                        },
                        severity: Status::Pass,
                    },
                ));
            }
        }
        // Reports that stopped.
        if let Some(stopped) = plan.stopped_ago {
            let days = stopped - 1;
            if days > STOPPED_AFTER_DAYS {
                let e = (DAYS - (stopped - STOPPED_AFTER_DAYS - 1)) as usize;
                out.push((
                    d,
                    at(rng, e.min(DAYS as usize - 1)),
                    AlertEvent {
                        kind: AlertKind::IssueOpened {
                            issue: AlertedIssue::ReportsStopped,
                            mail: None,
                            stopped: Some(StoppedFigures {
                                days: STOPPED_AFTER_DAYS + 1,
                                threshold_days: STOPPED_AFTER_DAYS,
                            }),
                        },
                        severity: Status::Warn,
                    },
                ));
            }
        }
    }

    // Record changes, on two domains that have had reports for a while.
    let spf_change = plans
        .iter()
        .position(|p| p.role == Role::Shaped && !p.esps.is_empty() && p.shape.first_ago > 40);
    if let Some(d) = spf_change {
        let plan = &plans[d];
        let added = format!("include:{}", services.esps[plan.esps[0].0].include);
        let severity = match plan.shape.dns {
            DnsClass::NoneOverLimit => Status::Fail,
            DnsClass::RejectSoftfail | DnsClass::NoneNeutral | DnsClass::Quarantine => Status::Warn,
            _ => Status::Neutral,
        };
        out.push((
            d,
            at(rng, (DAYS - 25) as usize),
            AlertEvent {
                kind: AlertKind::DnsRecordChanged {
                    check: CheckKind::Spf,
                    change: RecordDelta::SpfChanged {
                        added: vec![added],
                        removed: Vec::new(),
                        lookups_before: 1,
                        lookups_after: 2,
                    },
                },
                severity,
            },
        ));
    }
    let dkim_change = plans.iter().position(|p| {
        p.role == Role::Shaped
            && p.selectors.first() == Some(&"selector1")
            && p.shape.dns == DnsClass::None
            && p.shape.first_ago > 20
    });
    if let Some(d) = dkim_change {
        out.push((
            d,
            at(rng, (DAYS - 9) as usize),
            AlertEvent {
                kind: AlertKind::DnsRecordChanged {
                    check: CheckKind::Dkim,
                    change: RecordDelta::DkimKeyAppeared {
                        selector: "selector2".into(),
                    },
                },
                severity: Status::Neutral,
            },
        ));
    }
    out.sort_by_key(|(d, at, _)| (*at, *d));
    out
}
