//! Alerts: what changed about a domain between two evaluations.
//!
//! Marcan alerts on state changes, never on a state that persists. Each
//! evaluation reduces a domain to its [`AlertState`] (the judged facts alerts
//! compare); [`alert_events`] compares the previous state with the current
//! one and says what changed, as [`AlertEvent`]s. The first evaluation of a
//! domain is its baseline and raises nothing, and so is the first DNS result
//! or the first report of a domain evaluated before: a fact that was never
//! measured cannot have changed.
//!
//! What changed in the DNS records themselves is measured by `dmarc-dns`
//! (it diffs two stored results into [`RecordChange`]s, facts in the
//! vocabulary below); this module judges each change worth an event and
//! picks its colour. A change of the DMARC policy (`p`, `sp`, `pct`) is
//! read from that diff only, never from the reports: receivers report the
//! policy differently (one leaves `sp` out, another fills it in) and late,
//! so two reports differing is no change of the domain's.
//!
//! Marking a domain parked (or no longer parked) changes what it is judged
//! against, not the domain: its issues are swapped (`not_enforcing` for
//! `parked_not_rejecting`, ...). That is one [`AlertKind::ParkedChanged`]
//! event, and the evaluation it happens in is a new baseline for everything
//! that follows from the flag (issues, health, readiness); the records in
//! DNS still compare.
//!
//! Only changes a person acts on are events: a DNS record or the DMARC
//! policy changed, health changed, delivered failures opened or cleared,
//! reports stopped or resumed ([`AlertedIssue`]), the next policy
//! step came within reach, the domain was parked or unparked. The other
//! issues (DKIM coverage, `p=none`, the parked ones, the DNS grade) stay on
//! the dashboard and raise no event, and neither does the DNS grade moving
//! (health and the record changes cover it): [`AlertKind`] has no variant
//! for them.

mod catalogue;
mod events;
mod state;

pub use catalogue::{
    catalogue_entry, catalogue_id, worst_first, AlertGroup, CatalogueKind, ALERT_CATALOGUE,
};
pub use events::{alert_events, AlertEvent, AlertKind, AlertedIssue};
pub use state::{
    alert_state, AlertFigures, AlertState, BimiRecordFacts, DeliveredFigures, DkimKeyFacts,
    DkimSelectorFacts, DmarcRecordFacts, DnsRecords, DnsState, MtaStsPolicyFacts,
    MtaStsRecordFacts, NewSender, Observed, OpenIssue, ReadinessEvidence, RecordChange,
    RecordDelta, SpfRecordFacts, StoppedFigures, TlsRptRecordFacts, ALERT_FORMAT,
    NEW_SENDER_MIN_FAILURES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{CheckKind, Severity};
    use crate::domain::IssueKind;
    use crate::mail::{EnforcementState, HealthState};
    use crate::readiness::PolicyStep;
    use crate::status::Status;
    use std::collections::{BTreeMap, BTreeSet};

    /// One kind of every variant, every issue opening and clearing, and a
    /// record change of every check.
    fn every_kind() -> Vec<AlertKind> {
        let mut kinds = Vec::new();
        for issue in AlertedIssue::ALL {
            kinds.push(AlertKind::IssueOpened {
                issue,
                mail: None,
                stopped: None,
            });
            kinds.push(AlertKind::IssueResolved {
                issue,
                mail: None,
                stopped: None,
            });
        }
        kinds.push(AlertKind::HealthChanged {
            from: HealthState::Protected,
            to: HealthState::Warning,
            mail: None,
        });
        for check in [
            CheckKind::Dmarc,
            CheckKind::Spf,
            CheckKind::Dkim,
            CheckKind::MtaSts,
            CheckKind::TlsRpt,
            CheckKind::Bimi,
        ] {
            kinds.push(AlertKind::DnsRecordChanged {
                check,
                change: RecordDelta::Published,
            });
        }
        kinds.push(AlertKind::PolicyChanged {
            from: dmarc("reject", None, 100),
            to: dmarc("none", None, 100),
        });
        kinds.push(AlertKind::ReadinessReached {
            next: PolicyStep::Reject,
            evidence: None,
        });
        kinds.push(AlertKind::ParkedChanged { parked: true });
        kinds.push(AlertKind::ParkedChanged { parked: false });
        kinds.push(AlertKind::NewFailingSender {
            sender: sender("192.0.2.1", 10, 0),
            window_days: 30,
        });
        kinds
    }

    #[test]
    fn every_raised_kind_is_in_the_catalogue_once() {
        let kinds = every_kind();
        let ids: Vec<&str> = kinds.iter().map(catalogue_id).collect();
        // Each kind is a catalogue entry, and each entry is some kind's.
        assert!(ids.iter().all(|id| catalogue_entry(id).is_some()));
        let catalogue: BTreeSet<&str> = ALERT_CATALOGUE.iter().map(|e| e.id()).collect();
        assert_eq!(catalogue.len(), ALERT_CATALOGUE.len(), "ids repeat");
        assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>(), catalogue);
        // Every AlertKind variant is covered by every_kind().
        let names: BTreeSet<&str> = kinds.iter().map(AlertKind::name).collect();
        assert_eq!(names, AlertKind::NAMES.into_iter().collect());
        // An id starts with its kind's name, and each line is one sentence.
        for (kind, id) in kinds.iter().zip(&ids) {
            assert!(id.starts_with(kind.name()), "{id}");
        }
        // A kind's catalogue entry is its own, in the group of its id.
        for kind in &kinds {
            let entry = CatalogueKind::of(kind);
            assert_eq!(catalogue_entry(entry.id()), Some(entry));
        }
    }

    fn dmarc(p: &str, sp: Option<&str>, pct: u32) -> DmarcRecordFacts {
        DmarcRecordFacts {
            p: Some(p.into()),
            sp: sp.map(Into::into),
            pct,
            rua: vec!["mailto:d@example.com".into()],
            adkim: "r".into(),
            aspf: "r".into(),
        }
    }

    fn records() -> DnsRecords {
        DnsRecords {
            dmarc: Observed::Absent,
            spf: Observed::Absent,
            dkim: vec![],
            mta_sts: Observed::Absent,
            tls_rpt: Observed::Absent,
            bimi: Observed::Absent,
        }
    }

    fn dns() -> DnsState {
        DnsState {
            checker_version: 1,
            worst: BTreeMap::new(),
            records: records(),
        }
    }

    fn base() -> AlertState {
        AlertState {
            has_reports: true,
            health: HealthState::Protected,
            issues: vec![],
            ready_for: None,
            dns: Some(dns()),
            parked: false,
            figures: None,
            new_failing_senders: Some(vec![]),
        }
    }

    fn sender(ip: &str, failed: i64, delivered_failures: i64) -> NewSender {
        NewSender {
            source_ip: ip.into(),
            name: None,
            failed,
            delivered_failures,
        }
    }

    /// `base()` with the senders new to the domain that alerts keep.
    fn with_new_senders(new_senders: Vec<NewSender>) -> AlertState {
        AlertState {
            new_failing_senders: Some(state::failing_new_senders(new_senders)),
            ..base()
        }
    }

    /// A sender new to the domain alerts from the threshold on, red when
    /// failing mail was delivered and amber when all of it was blocked.
    #[test]
    fn a_new_sender_alerts_from_the_threshold() {
        let below = with_new_senders(vec![sender("192.0.2.1", 9, 9)]);
        assert_eq!(below.new_failing_senders, Some(vec![]));
        assert!(alert_events(Some(&base()), &below, &[]).is_empty());

        let at = with_new_senders(vec![
            sender("192.0.2.2", NEW_SENDER_MIN_FAILURES, 0),
            sender("192.0.2.1", 25, 3),
        ]);
        let events = alert_events(Some(&below), &at, &[]);
        assert_eq!(
            events,
            [
                AlertEvent::new(
                    AlertKind::NewFailingSender {
                        sender: sender("192.0.2.1", 25, 3),
                        window_days: 30,
                    },
                    Status::Fail
                ),
                AlertEvent::new(
                    AlertKind::NewFailingSender {
                        sender: sender("192.0.2.2", 10, 0),
                        window_days: 30,
                    },
                    Status::Warn
                ),
            ]
        );
    }

    /// Once alerted, a sender is known: it does not repeat while it keeps
    /// failing (whatever its count), and leaving the set (no longer new, or
    /// its failures gone with retention) raises nothing.
    #[test]
    fn a_known_sender_is_silent_and_leaving_raises_nothing() {
        let alerted = with_new_senders(vec![sender("192.0.2.1", 12, 0)]);
        let more = with_new_senders(vec![sender("192.0.2.1", 400, 380)]);
        assert!(alert_events(Some(&alerted), &more, &[]).is_empty());
        let aged = with_new_senders(vec![]);
        assert!(alert_events(Some(&more), &aged, &[]).is_empty());
        // Another sender is still new.
        let other = with_new_senders(vec![
            sender("192.0.2.1", 400, 380),
            sender("198.51.100.7", 11, 0),
        ]);
        assert_eq!(
            kinds(&alert_events(Some(&more), &other, &[])),
            ["new_failing_sender"]
        );
    }

    /// No flood: the first evaluation, the first reports, an import's
    /// rebaseline and a state stored before senders were measured are all
    /// baselines.
    #[test]
    fn new_senders_start_from_a_baseline() {
        let failing = with_new_senders(vec![
            sender("192.0.2.1", 50, 50),
            sender("192.0.2.2", 30, 0),
        ]);
        assert!(alert_events(None, &failing, &[]).is_empty());
        let empty = AlertState {
            has_reports: false,
            health: HealthState::NoData,
            new_failing_senders: Some(vec![]),
            ..base()
        };
        assert!(alert_events(Some(&empty), &failing, &[]).is_empty());
        let rebaselined = base().reports_rebaselined();
        assert!(alert_events(Some(&rebaselined), &failing, &[]).is_empty());
        let unmeasured = AlertState {
            new_failing_senders: None,
            ..base()
        };
        assert!(alert_events(Some(&unmeasured), &failing, &[]).is_empty());
        // A stored state of 0.2.1 has no senders: it reads as unmeasured.
        let mut json = serde_json::to_value(base()).unwrap();
        json.as_object_mut().unwrap().remove("new_failing_senders");
        let stored: AlertState = serde_json::from_value(json).unwrap();
        assert_eq!(stored.new_failing_senders, None);
        // From the baseline on, a sender joining is new.
        let joined = with_new_senders(vec![
            sender("192.0.2.1", 50, 50),
            sender("192.0.2.2", 30, 0),
            sender("192.0.2.3", 10, 1),
        ]);
        let events = alert_events(Some(&failing), &joined, &[]);
        assert_eq!(kinds(&events), ["new_failing_sender"]);
    }

    fn issue(kind: IssueKind, status: Status) -> OpenIssue {
        OpenIssue { kind, status }
    }

    fn kinds(events: &[AlertEvent]) -> Vec<&'static str> {
        events.iter().map(|e| e.kind.name()).collect()
    }

    #[test]
    fn no_previous_state_raises_nothing() {
        let mut current = base();
        current.issues = vec![issue(IssueKind::DeliveredFailures, Status::Fail)];
        current.health = HealthState::Critical;
        current.ready_for = Some(PolicyStep::Reject);
        let change = RecordChange {
            check: CheckKind::Spf,
            change: RecordDelta::Published,
        };
        assert!(alert_events(None, &current, &[change]).is_empty());
    }

    #[test]
    fn an_unchanged_state_raises_nothing() {
        let state = AlertState {
            issues: vec![issue(IssueKind::DeliveredFailures, Status::Fail)],
            ..base()
        };
        assert!(alert_events(Some(&state), &state, &[]).is_empty());
    }

    #[test]
    fn the_alerted_issues_open_and_resolve() {
        for kind in AlertedIssue::ALL {
            let before = base();
            let after = AlertState {
                issues: vec![issue(kind.kind(), Status::Warn)],
                ..base()
            };
            assert_eq!(
                alert_events(Some(&before), &after, &[]),
                [AlertEvent::new(
                    AlertKind::IssueOpened {
                        issue: kind,
                        mail: None,
                        stopped: None
                    },
                    Status::Warn
                )],
                "{kind:?} opened"
            );
            assert_eq!(
                alert_events(Some(&after), &before, &[]),
                [AlertEvent::new(
                    AlertKind::IssueResolved {
                        issue: kind,
                        mail: None,
                        stopped: None
                    },
                    Status::Pass
                )],
                "{kind:?} resolved"
            );
        }
    }

    /// DKIM coverage, `p=none`, the parked issues and the DNS grade stay on
    /// the dashboard: opening or clearing them is no event.
    #[test]
    fn the_other_issues_raise_nothing() {
        let others: Vec<IssueKind> = IssueKind::ALL
            .into_iter()
            .filter(|k| AlertedIssue::of(*k).is_none())
            .collect();
        assert_eq!(others.len(), 5);
        for kind in others {
            let after = AlertState {
                issues: vec![issue(kind, Status::Fail)],
                ..base()
            };
            assert!(
                alert_events(Some(&base()), &after, &[]).is_empty(),
                "{kind:?}"
            );
            assert!(
                alert_events(Some(&after), &base(), &[]).is_empty(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_persisting_issue_that_changes_colour_is_no_event() {
        let before = AlertState {
            issues: vec![issue(IssueKind::DeliveredFailures, Status::Warn)],
            ..base()
        };
        let after = AlertState {
            issues: vec![issue(IssueKind::DeliveredFailures, Status::Fail)],
            ..base()
        };
        assert!(alert_events(Some(&before), &after, &[]).is_empty());
    }

    #[test]
    fn health_changes_between_measured_states_only() {
        let before = base();
        let after = AlertState {
            health: HealthState::Warning,
            ..base()
        };
        assert_eq!(
            alert_events(Some(&before), &after, &[]),
            [AlertEvent::new(
                AlertKind::HealthChanged {
                    from: HealthState::Protected,
                    to: HealthState::Warning,
                    mail: None,
                },
                Status::Warn
            )]
        );
        let recovered = alert_events(Some(&after), &before, &[]);
        assert_eq!(recovered[0].severity, Status::Pass);

        let quiet = AlertState {
            health: HealthState::NoData,
            ..base()
        };
        assert!(alert_events(Some(&before), &quiet, &[]).is_empty());
        assert!(alert_events(Some(&quiet), &before, &[]).is_empty());
    }

    #[test]
    fn the_first_reports_are_a_baseline() {
        let empty = AlertState {
            has_reports: false,
            health: HealthState::NoData,
            ..base()
        };
        let first = AlertState {
            health: HealthState::Critical,
            issues: vec![
                issue(IssueKind::ReportsStopped, Status::Warn),
                issue(IssueKind::DeliveredFailures, Status::Fail),
            ],
            ready_for: Some(PolicyStep::Quarantine),
            ..base()
        };
        assert!(alert_events(Some(&empty), &first, &[]).is_empty());
        // Every report deleted (retention): nothing measured, nothing resolved.
        assert!(alert_events(Some(&first), &empty, &[]).is_empty());
    }

    /// After an import, the reports' parts are a new baseline; DNS still
    /// compares.
    #[test]
    fn a_rebaselined_state_compares_only_its_dns() {
        let before = AlertState {
            issues: vec![issue(IssueKind::ReportsStopped, Status::Warn)],
            ..base()
        };
        let after = AlertState {
            health: HealthState::Critical,
            ..base()
        };
        let spf = [RecordChange {
            check: CheckKind::Spf,
            change: RecordDelta::Published,
        }];
        assert_eq!(
            kinds(&alert_events(Some(&before), &after, &spf)),
            ["issue_resolved", "health_changed", "dns_record_changed"]
        );
        let events = alert_events(Some(&before.reports_rebaselined()), &after, &spf);
        assert_eq!(kinds(&events), ["dns_record_changed"]);
    }

    #[test]
    fn the_first_dns_result_and_a_new_checker_are_baselines() {
        let no_dns = AlertState {
            dns: None,
            ..base()
        };
        let change = [RecordChange {
            check: CheckKind::Spf,
            change: RecordDelta::Removed,
        }];
        assert!(alert_events(Some(&no_dns), &base(), &change).is_empty());

        let rechecked = AlertState {
            dns: Some(DnsState {
                checker_version: 2,
                ..dns()
            }),
            ..base()
        };
        assert!(alert_events(Some(&base()), &rechecked, &change).is_empty());
    }

    #[test]
    fn record_changes_take_the_colour_of_the_check() {
        let before = base();
        let mut after = base();
        after.dns.as_mut().unwrap().worst = BTreeMap::from([
            (CheckKind::Spf, Severity::Critical),
            (CheckKind::Dkim, Severity::Ok),
            (CheckKind::Dmarc, Severity::Warning),
        ]);
        let changes = [
            RecordChange {
                check: CheckKind::Spf,
                change: RecordDelta::SpfChanged {
                    added: vec!["include:new.example".into()],
                    removed: vec![],
                    lookups_before: 9,
                    lookups_after: 11,
                },
            },
            RecordChange {
                check: CheckKind::Dkim,
                change: RecordDelta::DkimKeyChanged {
                    selector: "s1".into(),
                },
            },
            RecordChange {
                check: CheckKind::Dmarc,
                change: RecordDelta::Published,
            },
            RecordChange {
                check: CheckKind::Bimi,
                change: RecordDelta::Removed,
            },
        ];
        let events = alert_events(Some(&before), &after, &changes);
        let colours: Vec<Status> = events.iter().map(|e| e.severity).collect();
        assert_eq!(
            colours,
            [Status::Fail, Status::Neutral, Status::Warn, Status::Neutral]
        );
        assert_eq!(
            events[1].kind,
            AlertKind::DnsRecordChanged {
                check: CheckKind::Dkim,
                change: RecordDelta::DkimKeyChanged {
                    selector: "s1".into()
                }
            }
        );
    }

    fn dmarc_change(before: DmarcRecordFacts, after: DmarcRecordFacts) -> RecordChange {
        RecordChange {
            check: CheckKind::Dmarc,
            change: RecordDelta::DmarcChanged { before, after },
        }
    }

    #[test]
    fn policy_changes_are_coloured_by_direction() {
        let change = |from: DmarcRecordFacts, to: DmarcRecordFacts| {
            let events = alert_events(Some(&base()), &base(), &[dmarc_change(from, to)]);
            assert_eq!(kinds(&events), ["policy_changed"]);
            events[0].severity
        };
        assert_eq!(
            change(dmarc("none", None, 100), dmarc("quarantine", None, 100)),
            Status::Pass
        );
        assert_eq!(
            change(dmarc("reject", None, 100), dmarc("quarantine", None, 100)),
            Status::Warn
        );
        assert_eq!(
            change(dmarc("reject", None, 100), dmarc("reject", None, 50)),
            Status::Warn
        );
        assert_eq!(
            change(dmarc("quarantine", None, 100), dmarc("none", None, 100)),
            Status::Fail
        );
        assert_eq!(
            change(
                dmarc("reject", None, 100),
                dmarc("reject", Some("none"), 100)
            ),
            Status::Warn
        );
        // `sp=` written out as `p=` is another record, but no stricter.
        assert_eq!(
            change(
                dmarc("reject", None, 100),
                dmarc("reject", Some("reject"), 100)
            ),
            Status::Neutral
        );
    }

    /// The policy comes from the DMARC record's diff alone: one event per
    /// changed record, the policy change when `p`, `sp` or `pct` moved
    /// (whatever else moved with them), else a record change.
    #[test]
    fn a_dmarc_record_change_is_one_event() {
        let before = dmarc("none", None, 100);
        let mut after = dmarc("quarantine", None, 100);
        after.rua = vec!["mailto:other@example.com".into()];
        let events = alert_events(
            Some(&base()),
            &base(),
            &[dmarc_change(before.clone(), after.clone())],
        );
        assert_eq!(
            events,
            [AlertEvent::new(
                AlertKind::PolicyChanged {
                    from: before.clone(),
                    to: after.clone()
                },
                Status::Pass
            )]
        );

        let mut rua_only = before.clone();
        rua_only.rua = after.rua;
        let events = alert_events(Some(&base()), &base(), &[dmarc_change(before, rua_only)]);
        assert_eq!(kinds(&events), ["dns_record_changed"]);

        // Nothing changed in DNS: no policy event, whatever reports said.
        assert!(alert_events(Some(&base()), &base(), &[]).is_empty());
    }

    /// The owner's case: a domain at `p=none` without enough DKIM is marked
    /// parked while mail passes DMARC for it. Its issues are swapped and its
    /// health and readiness move with the flag; one event says what
    /// happened.
    fn sending_and_parked() -> (AlertState, AlertState) {
        let sending = AlertState {
            health: HealthState::Protected,
            issues: vec![
                issue(IssueKind::NotEnforcing, Status::Fail),
                issue(IssueKind::DkimCoverage, Status::Warn),
            ],
            ready_for: Some(PolicyStep::Quarantine),
            ..base()
        };
        let parked = AlertState {
            health: HealthState::Critical,
            issues: vec![
                issue(IssueKind::ParkedPassingMail, Status::Fail),
                issue(IssueKind::ParkedNotRejecting, Status::Fail),
            ],
            ready_for: None,
            parked: true,
            ..base()
        };
        (sending, parked)
    }

    #[test]
    fn parking_a_domain_is_one_event() {
        let (sending, parked) = sending_and_parked();
        assert_eq!(
            alert_events(Some(&sending), &parked, &[]),
            [AlertEvent::new(
                AlertKind::ParkedChanged { parked: true },
                Status::Neutral
            )]
        );
        // The next evaluation compares with the parked state: nothing more.
        assert!(alert_events(Some(&parked), &parked, &[]).is_empty());
    }

    #[test]
    fn unparking_a_domain_is_one_event() {
        let (sending, parked) = sending_and_parked();
        assert_eq!(
            alert_events(Some(&parked), &sending, &[]),
            [AlertEvent::new(
                AlertKind::ParkedChanged { parked: false },
                Status::Neutral
            )]
        );
        assert!(alert_events(Some(&sending), &sending, &[]).is_empty());
    }

    /// The records in DNS do not follow from the flag: a change still counts,
    /// in the evaluation the flag changes in and after it.
    #[test]
    fn a_parked_domain_still_raises_record_changes() {
        let (sending, parked) = sending_and_parked();
        let spf = [RecordChange {
            check: CheckKind::Spf,
            change: RecordDelta::Published,
        }];
        assert_eq!(
            kinds(&alert_events(Some(&sending), &parked, &spf)),
            ["parked_changed", "dns_record_changed"]
        );
        assert_eq!(
            kinds(&alert_events(Some(&parked), &parked, &spf)),
            ["dns_record_changed"]
        );
        let policy = [dmarc_change(
            dmarc("none", None, 100),
            dmarc("reject", None, 100),
        )];
        assert_eq!(
            kinds(&alert_events(Some(&sending), &parked, &policy)),
            ["parked_changed", "policy_changed"]
        );
    }

    #[test]
    fn readiness_reached_once() {
        let before = base();
        let ready = AlertState {
            ready_for: Some(PolicyStep::Reject),
            ..base()
        };
        assert_eq!(
            alert_events(Some(&before), &ready, &[]),
            [AlertEvent::new(
                AlertKind::ReadinessReached {
                    next: PolicyStep::Reject,
                    evidence: None,
                },
                Status::Pass
            )]
        );
        assert!(alert_events(Some(&ready), &ready, &[]).is_empty());
        // No longer ready is not an alert: the issues say why.
        assert!(alert_events(Some(&ready), &before, &[]).is_empty());
    }

    fn figures(delivered: i64, delivered_failures: i64, stopped: Option<i64>) -> AlertFigures {
        AlertFigures {
            window_days: 30,
            delivered,
            delivered_failures,
            blocked: 4,
            reports_stopped_days: stopped,
            reports_stopped_after_days: 7,
            policy: EnforcementState::Monitor,
            policy_days: Some(30),
            policy_messages: 1204,
            delivered_compliance_pct: Some(99.6),
        }
    }

    /// Each event carries the numbers its sentence names: the window's
    /// mail against the threshold of the event's colour, how long reports
    /// were missing against the setting, the evidence of a step reached.
    #[test]
    fn events_carry_the_numbers_they_are_based_on() {
        let before = AlertState {
            figures: Some(figures(1000, 2, Some(9))),
            issues: vec![issue(IssueKind::ReportsStopped, Status::Warn)],
            ..base()
        };
        let after = AlertState {
            health: HealthState::Critical,
            issues: vec![issue(IssueKind::DeliveredFailures, Status::Fail)],
            ready_for: Some(PolicyStep::Quarantine),
            figures: Some(figures(200, 12, None)),
            ..base()
        };
        let events = alert_events(Some(&before), &after, &[]);
        let failing = DeliveredFigures {
            window_days: 30,
            delivered: 200,
            delivered_failures: 12,
            blocked: 4,
            failure_pct: Some(6.0),
            threshold_pct: 5.0,
        };
        assert_eq!(
            events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(),
            [
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: Some(failing),
                    stopped: None,
                },
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: Some(StoppedFigures {
                        days: 9,
                        threshold_days: 7
                    }),
                },
                AlertKind::HealthChanged {
                    from: HealthState::Protected,
                    to: HealthState::Critical,
                    mail: Some(failing),
                },
                AlertKind::ReadinessReached {
                    next: PolicyStep::Quarantine,
                    evidence: Some(ReadinessEvidence {
                        policy: EnforcementState::Monitor,
                        days: 30,
                        messages: 1204,
                        delivered_compliance_pct: Some(99.6),
                    }),
                },
            ]
        );
        // A warning rate is measured against the warning threshold, and a
        // cleared issue too.
        let warn = figures(1000, 20, None).delivered(Status::Warn);
        assert_eq!((warn.failure_pct, warn.threshold_pct), (Some(2.0), 1.0));
        assert_eq!(
            figures(0, 0, None).delivered(Status::Pass).failure_pct,
            None
        );
        // Opened stopped reports name the days now.
        let stopped = AlertState {
            issues: vec![issue(IssueKind::ReportsStopped, Status::Warn)],
            figures: Some(figures(1000, 0, Some(9))),
            ..base()
        };
        let opened = alert_events(Some(&base()), &stopped, &[]);
        assert_eq!(
            opened[0].kind,
            AlertKind::IssueOpened {
                issue: AlertedIssue::ReportsStopped,
                mail: None,
                stopped: Some(StoppedFigures {
                    days: 9,
                    threshold_days: 7
                }),
            }
        );
    }

    #[test]
    fn names_match_the_serialized_tags() {
        let samples = [
            AlertKind::IssueOpened {
                issue: AlertedIssue::DeliveredFailures,
                mail: None,
                stopped: None,
            },
            AlertKind::IssueResolved {
                issue: AlertedIssue::ReportsStopped,
                mail: None,
                stopped: None,
            },
            AlertKind::HealthChanged {
                from: HealthState::Protected,
                to: HealthState::Critical,
                mail: None,
            },
            AlertKind::DnsRecordChanged {
                check: CheckKind::Spf,
                change: RecordDelta::Removed,
            },
            AlertKind::PolicyChanged {
                from: dmarc("reject", None, 100),
                to: dmarc("none", None, 100),
            },
            AlertKind::ReadinessReached {
                next: PolicyStep::Reject,
                evidence: None,
            },
            AlertKind::ParkedChanged { parked: true },
            AlertKind::NewFailingSender {
                sender: sender("192.0.2.1", 10, 0),
                window_days: 30,
            },
        ];
        let names: Vec<&str> = samples.iter().map(AlertKind::name).collect();
        assert_eq!(names, AlertKind::NAMES);
        for sample in samples {
            let json = serde_json::to_value(AlertEvent::new(sample.clone(), Status::Pass)).unwrap();
            assert_eq!(json["kind"], sample.name());
            let back: AlertEvent = serde_json::from_value(json).unwrap();
            assert_eq!(back.kind, sample);
        }
    }

    #[test]
    fn states_round_trip_as_json() {
        let mut state = base();
        state.issues = vec![issue(IssueKind::ReportsStopped, Status::Warn)];
        state.dns.as_mut().unwrap().records.dkim = vec![DkimSelectorFacts {
            selector: "s1".into(),
            key: Observed::Published(DkimKeyFacts {
                key_id: "abc".into(),
                revoked: false,
            }),
        }];
        state
            .dns
            .as_mut()
            .unwrap()
            .worst
            .insert(CheckKind::MtaSts, Severity::Info);
        state.new_failing_senders = Some(vec![NewSender {
            name: Some("Example Mail".into()),
            ..sender("192.0.2.1", 10, 2)
        }]);
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<AlertState>(&json).unwrap(), state);
    }

    #[test]
    fn worst_first_is_stable() {
        let mut items = vec![
            ("a", Status::Pass),
            ("b", Status::Fail),
            ("c", Status::Neutral),
            ("d", Status::Fail),
            ("e", Status::Warn),
        ];
        worst_first(&mut items, |i| i.1);
        let order: Vec<&str> = items.iter().map(|i| i.0).collect();
        assert_eq!(order, ["b", "d", "e", "a", "c"]);
    }
}
