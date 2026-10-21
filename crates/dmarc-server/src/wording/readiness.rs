//! A domain's policy readiness in words: the verdict as a lead and its
//! reason ([`verdict`]), the header's short form ([`headline`]), the rules
//! of the next step ([`evidence`]), the records to publish ([`steps`]) and
//! a parked domain's own lines ([`parked`]). Which state applies, which rule
//! is met and the plan are `dmarc_assess::policy_readiness`'s; the numbers
//! are its constants. The domain page and the client report both read the
//! verdict from here, so a client and the operator read the same sentence.

use super::alerts::{policy_name, ready_evidence, step_name};
use super::{count_noun, format_count, format_pct, one_or_many};
use crate::view::readiness::{EvidenceRow, ParkedReadiness};
use dmarc_assess::{
    NotApplicableReason, PolicyReadiness, PolicyStep, PolicyTarget, ReadinessReason,
    ReadinessState, RATE_PASS_MIN_PCT, READINESS_MIN_MESSAGES, READINESS_MIN_POLICY_SHARE_PCT,
    READINESS_WINDOW_DAYS,
};

/// The verdict: `lead` carries it (coloured by the readiness status),
/// `detail` says why; read as "lead: detail".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub lead: String,
    pub detail: String,
}

/// The share of the window's mail still evaluated under another policy.
fn old_policy_share(r: &PolicyReadiness) -> String {
    format_pct(100.0 - r.window.policy_share_pct.unwrap_or(0.0))
}

/// Why the next step has to wait, from the window's figures.
fn reason(reason: ReadinessReason, r: &PolicyReadiness) -> String {
    let days = READINESS_WINDOW_DAYS;
    let under = policy_name(r.window.policy);
    match reason {
        ReadinessReason::TooLittleMail => format!(
            "{} of {} messages under {under} over {days} days, too little mail to judge.",
            format_count(r.window.policy_messages),
            format_count(READINESS_MIN_MESSAGES)
        ),
        ReadinessReason::NotEnoughHistory { .. } => format!(
            "reports cover {} of {days} days under {under} so far.",
            r.history_days.unwrap_or(0)
        ),
        ReadinessReason::OldPolicyApplied => format!(
            "receivers still apply the old policy to {} of mail.",
            old_policy_share(r)
        ),
        ReadinessReason::BelowThreshold => match r.window.delivered_compliance.pct {
            None => format!("no delivered mail over {days} days to measure compliance."),
            Some(pct) => format!(
                "{} delivered compliance over {days} days (needs {RATE_PASS_MIN_PCT:.0}%).",
                format_pct(pct)
            ),
        },
    }
}

/// The verdict in one sentence, whatever the state.
pub fn verdict(r: &PolicyReadiness) -> Verdict {
    let days = READINESS_WINDOW_DAYS;
    let (lead, detail) = match r.state {
        ReadinessState::Ready { next } => (
            format!("Ready for {}", step_name(next)),
            format!(
                "{}.",
                ready_evidence(
                    r.history_days.unwrap_or(days),
                    r.window.policy_messages,
                    r.window.policy,
                    r.window.delivered_compliance.pct
                )
            ),
        ),
        ReadinessState::RaisePct {
            pct,
            testing: false,
        } => (
            "Raise pct to 100 first".to_owned(),
            format!("the policy applies to only {pct}% of the mail that fails DMARC."),
        ),
        ReadinessState::RaisePct {
            pct,
            testing: true,
        } => (
            if pct < 100 {
                "Remove t=y and raise pct to 100 first"
            } else {
                "Remove t=y first"
            }
            .to_owned(),
            "the record is in testing mode (t=y), so receivers that follow DMARCbis do not apply the policy.".to_owned(),
        ),
        ReadinessState::NotReady { next, reason: why } => (
            format!("Not ready for {} yet", step_name(next)),
            reason(why, r),
        ),
        ReadinessState::Enforced => (
            "Enforced".to_owned(),
            "p=reject applies to all failing mail, so there is no next step.".to_owned(),
        ),
        ReadinessState::NotApplicable {
            reason: NotApplicableReason::NoPolicy,
        } => (
            "Nothing to judge yet".to_owned(),
            "no DMARC policy is known for this domain.".to_owned(),
        ),
        ReadinessState::NotApplicable {
            reason: NotApplicableReason::Parked,
        } => (
            "Publish p=reject now".to_owned(),
            "the domain is marked as parked.".to_owned(),
        ),
    };
    Verdict { lead, detail }
}

/// The verdict in a few words under the domain's title ("Ready for
/// p=quarantine", "Enforced"); `None` when the badges say it all (no policy
/// known, a parked domain).
pub fn headline(r: &PolicyReadiness) -> Option<String> {
    match r.state {
        ReadinessState::NotApplicable { .. } => None,
        _ => Some(verdict(r).lead),
    }
}

/// The rules of the next step, in the order the verdict reads them: the
/// messages and days under the current policy, how much of the window's
/// mail was evaluated under it, and delivered compliance over all of it.
/// Empty when the rules do not decide the state.
pub fn evidence(r: &PolicyReadiness) -> Vec<EvidenceRow> {
    if !r.rules_decide() {
        return Vec::new();
    }
    let w = &r.window;
    let c = &r.criteria;
    let days = READINESS_WINDOW_DAYS;
    let policy = policy_name(w.policy);
    vec![
        EvidenceRow {
            label: "Messages".to_owned(),
            value: format!("{} under {policy}", format_count(w.policy_messages)),
            rule: Some(format!("needs {}", format_count(READINESS_MIN_MESSAGES))),
            met: c.enough_mail,
            hint: None,
        },
        EvidenceRow {
            label: "Report history".to_owned(),
            value: match r.history_days {
                None => format!("No reports under {policy}"),
                Some(n) => format!("{} of {days} days under {policy}", format_count(n)),
            },
            rule: None,
            met: c.full_history,
            hint: None,
        },
        EvidenceRow {
            label: format!("Mail under {policy}"),
            value: w
                .policy_share_pct
                .map_or_else(|| "No mail".to_owned(), format_pct),
            rule: Some(format!("needs {READINESS_MIN_POLICY_SHARE_PCT:.0}%")),
            met: c.policy_share_met,
            hint: (!c.policy_share_met && w.policy_share_pct.is_some()).then(|| {
                format!(
                    "{} of mail was still evaluated under {}.",
                    old_policy_share(r),
                    policy_name(w.other_policy)
                )
            }),
        },
        EvidenceRow {
            label: "Delivered compliance".to_owned(),
            value: w
                .delivered_compliance
                .pct
                .map_or_else(|| "Nothing delivered".to_owned(), format_pct),
            rule: Some(format!("needs {RATE_PASS_MIN_PCT:.0}%")),
            met: c.compliance_met,
            hint: None,
        },
    ]
}

/// The plan's records, each named and placed: the first "Publish now" when
/// the verdict recommends it (and then the record to publish), "Once ready"
/// when not ready; every later one waits for its own verdict.
pub fn steps(r: &PolicyReadiness) -> Vec<(PolicyTarget, StepWords)> {
    let recommended = r.recommends_first_step();
    r.plan
        .iter()
        .enumerate()
        .map(|(i, target)| {
            let words = StepWords {
                policy: step_name(target.p).to_owned(),
                when: match (i, recommended) {
                    (0, true) => "Publish now",
                    (0, false) => "Once ready",
                    _ => "Then, once ready again",
                }
                .to_owned(),
                primary: i == 0 && recommended,
            };
            (*target, words)
        })
        .collect()
}

/// A step of the plan in words (see [`PlanStep`](crate::view::readiness::PlanStep)).
pub struct StepWords {
    pub policy: String,
    pub when: String,
    pub primary: bool,
}

/// Before `p=reject` on a quarantining domain: how much mail the policy
/// already stopped in the window, since that mail is refused, not sent to
/// spam, once the step is taken. `None` in any other state, or when nothing
/// was blocked.
pub fn blocked_note(r: &PolicyReadiness) -> Option<String> {
    let n = r.window.blocked;
    let ready_for_reject = matches!(
        r.state,
        ReadinessState::Ready {
            next: PolicyStep::Reject
        }
    );
    (ready_for_reject && n > 0).then(|| {
        format!(
            "{} {} blocked by policy in the last {} days. Check the senders below before moving to {}.",
            count_noun(n, "message"),
            one_or_many(n, "was", "were"),
            READINESS_WINDOW_DAYS,
            step_name(PolicyStep::Reject)
        )
    })
}

/// A parked domain's verdict and the window's mail, with a record or
/// without one (it publishes a new one at `p=reject`); `None` in a state
/// the parked layout does not apply to.
pub fn parked(r: &PolicyReadiness) -> Option<ParkedReadiness> {
    let reject = step_name(PolicyStep::Reject);
    let (lead, detail) = match r.state {
        ReadinessState::Enforced => ("Enforced:".to_owned(), format!("{reject}, nothing to do.")),
        ReadinessState::NotApplicable {
            reason: NotApplicableReason::Parked,
        } => (
            format!("Publish {reject} now."),
            "This domain is marked as parked: it sends no mail, so rejecting everything loses nothing. Leave sp= and np= out, or set both to reject, so its subdomains are refused too."
                .to_owned(),
        ),
        _ => return None,
    };
    Some(ParkedReadiness {
        lead,
        detail,
        mail_line: parked_mail_line(r),
    })
}

/// The line over a parked domain's mail in the window: every message
/// counts, failing or not, since a parked domain should send none. Mail
/// that passed DMARC is delivered under `p=reject` too, so the line says so
/// when there is any.
fn parked_mail_line(r: &PolicyReadiness) -> String {
    let days = READINESS_WINDOW_DAYS;
    let reject = step_name(PolicyStep::Reject);
    let (messages, compliant) = (r.window.messages, r.window.compliant);
    if messages == 0 {
        return format!("No mail was sent in the last {days} days.");
    }
    let sent = format!(
        "{} {} sent in the last {days} days although the domain is parked.",
        count_noun(messages, "message"),
        one_or_many(messages, "was", "were")
    );
    if compliant == 0 {
        return format!(
            "{sent} Under {reject} receivers refuse {}.",
            one_or_many(messages, "it", "all of them")
        );
    }
    format!(
        "{sent} Under {reject} receivers refuse the mail that fails DMARC; {} passed DMARC and {} still delivered.",
        count_noun(compliant, "message"),
        one_or_many(compliant, "is", "are")
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dmarc_assess::{EnforcementState, Rate, ReadinessCriteria, ReadinessWindow, Status};

    /// All of the window's mail under the current policy: `p=quarantine`
    /// before reject, else `p=none`.
    pub(crate) fn readiness(
        state: ReadinessState,
        messages: i64,
        pct: Option<f64>,
    ) -> PolicyReadiness {
        let next = match state {
            ReadinessState::Ready { next } | ReadinessState::NotReady { next, .. } => Some(next),
            _ => None,
        };
        PolicyReadiness {
            state,
            status: Status::Neutral,
            window: ReadinessWindow {
                since: 1_797_000_000,
                policy: if next == Some(PolicyStep::Reject) {
                    EnforcementState::Quarantine
                } else {
                    EnforcementState::Monitor
                },
                messages,
                policy_messages: messages,
                policy_first_report_at: Some(1_798_000_000),
                policy_share_pct: (messages > 0).then_some(100.0),
                other_policy: EnforcementState::Unknown,
                compliant: messages - 3,
                delivered_failures: 3,
                failed: 5,
                blocked: 2,
                forwarded_failures: 1,
                delivered_compliance: Rate {
                    pct,
                    status: Status::Pass,
                },
            },
            criteria: ReadinessCriteria {
                enough_mail: true,
                full_history: false,
                policy_share_met: true,
                compliance_met: true,
            },
            history_days: Some(15),
            history_full_at: Some(1_800_592_000),
            plan: Vec::new(),
        }
    }

    pub(crate) fn not_ready(next: PolicyStep, reason: ReadinessReason) -> ReadinessState {
        ReadinessState::NotReady { next, reason }
    }

    fn line(r: &PolicyReadiness) -> String {
        let v = verdict(r);
        format!("{}: {}", v.lead, v.detail)
    }

    const Q: PolicyStep = PolicyStep::Quarantine;
    const R: PolicyStep = PolicyStep::Reject;

    #[test]
    fn every_state_reads_as_a_sentence() {
        let ready = |next| ReadinessState::Ready { next };
        let mut ready_q = readiness(ready(Q), 4120, Some(99.8));
        // Ready means reports under the policy cover the whole window.
        ready_q.history_days = Some(30);
        assert_eq!(
            line(&ready_q),
            "Ready for p=quarantine: 30 days and 4,120 messages under p=none, 99.8% delivered compliance."
        );
        assert_eq!(
            line(&readiness(
                not_ready(Q, ReadinessReason::BelowThreshold),
                900,
                Some(97.1)
            )),
            "Not ready for p=quarantine yet: 97.1% delivered compliance over 30 days (needs 99%)."
        );
        assert_eq!(
            line(&readiness(
                not_ready(R, ReadinessReason::BelowThreshold),
                900,
                None
            )),
            "Not ready for p=reject yet: no delivered mail over 30 days to measure compliance."
        );
        assert_eq!(
            line(&readiness(not_ready(Q, ReadinessReason::TooLittleMail), 50, Some(99.8))),
            "Not ready for p=quarantine yet: 50 of 200 messages under p=none over 30 days, too little mail to judge."
        );
        let history = ReadinessReason::NotEnoughHistory {
            window_full_at: 1_800_000_000,
        };
        assert_eq!(
            line(&readiness(not_ready(R, history), 4120, Some(99.8))),
            "Not ready for p=reject yet: reports cover 15 of 30 days under p=quarantine so far."
        );
        let mut late = readiness(
            not_ready(R, ReadinessReason::OldPolicyApplied),
            4120,
            Some(99.8),
        );
        late.window.policy_share_pct = Some(88.0);
        assert_eq!(
            line(&late),
            "Not ready for p=reject yet: receivers still apply the old policy to 12.0% of mail."
        );
        let raise = |pct, testing| ReadinessState::RaisePct { pct, testing };
        assert_eq!(
            line(&readiness(raise(50, false), 4120, Some(99.8))),
            "Raise pct to 100 first: the policy applies to only 50% of the mail that fails DMARC."
        );
        assert_eq!(
            line(&readiness(raise(20, false), 4120, Some(99.8))),
            "Raise pct to 100 first: the policy applies to only 20% of the mail that fails DMARC."
        );
        assert_eq!(
            line(&readiness(raise(100, true), 4120, Some(99.8))),
            "Remove t=y first: the record is in testing mode (t=y), so receivers that follow DMARCbis do not apply the policy."
        );
        assert_eq!(
            line(&readiness(raise(50, true), 4120, Some(99.8))),
            "Remove t=y and raise pct to 100 first: the record is in testing mode (t=y), so receivers that follow DMARCbis do not apply the policy."
        );
        assert_eq!(
            line(&readiness(ReadinessState::Enforced, 4120, Some(99.8))),
            "Enforced: p=reject applies to all failing mail, so there is no next step."
        );
        let na = |reason| ReadinessState::NotApplicable { reason };
        assert_eq!(
            line(&readiness(na(NotApplicableReason::NoPolicy), 0, None)),
            "Nothing to judge yet: no DMARC policy is known for this domain."
        );
        assert_eq!(
            line(&readiness(na(NotApplicableReason::Parked), 0, None)),
            "Publish p=reject now: the domain is marked as parked."
        );
    }

    #[test]
    fn the_headline_is_the_lead_unless_the_badges_say_it() {
        let headline = |state| headline(&readiness(state, 4120, Some(99.8)));
        assert_eq!(
            headline(ReadinessState::Ready { next: Q }).as_deref(),
            Some("Ready for p=quarantine")
        );
        assert_eq!(
            headline(not_ready(Q, ReadinessReason::TooLittleMail)).as_deref(),
            Some("Not ready for p=quarantine yet")
        );
        assert_eq!(
            headline(ReadinessState::Enforced).as_deref(),
            Some("Enforced")
        );
        assert_eq!(
            headline(ReadinessState::NotApplicable {
                reason: NotApplicableReason::Parked
            }),
            None
        );
        assert_eq!(
            headline(ReadinessState::NotApplicable {
                reason: NotApplicableReason::NoPolicy
            }),
            None
        );
    }

    #[test]
    fn the_blocked_mail_is_counted_only_before_reject() {
        let mut r = readiness(ReadinessState::Ready { next: R }, 4120, Some(99.8));
        assert_eq!(
            blocked_note(&r).as_deref(),
            Some("2 messages were blocked by policy in the last 30 days. Check the senders below before moving to p=reject.")
        );
        r.window.blocked = 1;
        assert!(blocked_note(&r)
            .unwrap()
            .starts_with("1 message was blocked"));
        r.window.blocked = 0;
        assert_eq!(blocked_note(&r), None);
        let q = readiness(ReadinessState::Ready { next: Q }, 4120, Some(99.8));
        assert_eq!(blocked_note(&q), None);
        let not = readiness(
            not_ready(R, ReadinessReason::BelowThreshold),
            4120,
            Some(99.8),
        );
        assert_eq!(blocked_note(&not), None);
    }

    #[test]
    fn a_parked_domain_reads_the_instruction_then_its_mail() {
        let parked_state = ReadinessState::NotApplicable {
            reason: NotApplicableReason::Parked,
        };
        let p = parked(&readiness(parked_state, 0, None)).unwrap();
        assert_eq!(p.lead, "Publish p=reject now.");
        assert!(p.detail.ends_with(
            "Leave sp= and np= out, or set both to reject, so its subdomains are refused too."
        ));
        assert_eq!(p.mail_line, "No mail was sent in the last 30 days.");
        let enforced = parked(&readiness(ReadinessState::Enforced, 0, None)).unwrap();
        assert_eq!(
            (enforced.lead.as_str(), enforced.detail.as_str()),
            ("Enforced:", "p=reject, nothing to do.")
        );
        let no_policy = ReadinessState::NotApplicable {
            reason: NotApplicableReason::NoPolicy,
        };
        assert_eq!(parked(&readiness(no_policy, 0, None)), None);

        let mut spoofed = readiness(parked_state, 12, None);
        spoofed.window.compliant = 0;
        assert_eq!(
            parked(&spoofed).unwrap().mail_line,
            "12 messages were sent in the last 30 days although the domain is parked. Under p=reject receivers refuse all of them."
        );
        spoofed.window.compliant = 1;
        assert_eq!(
            parked(&spoofed).unwrap().mail_line,
            "12 messages were sent in the last 30 days although the domain is parked. Under p=reject receivers refuse the mail that fails DMARC; 1 message passed DMARC and is still delivered."
        );
    }

    fn rows(r: &PolicyReadiness) -> Vec<String> {
        evidence(r)
            .iter()
            .map(|row| match &row.rule {
                Some(rule) => format!("{}: {}, {rule} ({})", row.label, row.value, row.met),
                None => format!("{}: {} ({})", row.label, row.value, row.met),
            })
            .collect()
    }

    #[test]
    fn each_rule_reads_with_what_the_window_showed() {
        let history = ReadinessReason::NotEnoughHistory {
            window_full_at: 1_800_592_000,
        };
        let r = readiness(not_ready(Q, history), 4120, Some(99.8));
        assert_eq!(
            rows(&r),
            [
                "Messages: 4,120 under p=none, needs 200 (true)",
                "Report history: 15 of 30 days under p=none (false)",
                "Mail under p=none: 100.0%, needs 95% (true)",
                "Delivered compliance: 99.8%, needs 99% (true)",
            ]
        );
        assert!(evidence(&r).iter().all(|row| row.hint.is_none()));

        let mut r = readiness(
            not_ready(R, ReadinessReason::TooLittleMail),
            1000,
            Some(99.8),
        );
        r.window.policy_messages = 140;
        r.window.policy_share_pct = Some(14.0);
        r.window.other_policy = EnforcementState::Monitor;
        r.history_days = Some(0);
        r.criteria = ReadinessCriteria {
            enough_mail: false,
            full_history: false,
            policy_share_met: false,
            compliance_met: true,
        };
        assert_eq!(
            rows(&r),
            [
                "Messages: 140 under p=quarantine, needs 200 (false)",
                "Report history: 0 of 30 days under p=quarantine (false)",
                "Mail under p=quarantine: 14.0%, needs 95% (false)",
                "Delivered compliance: 99.8%, needs 99% (true)",
            ]
        );
        assert_eq!(
            evidence(&r)[2].hint.as_deref(),
            Some("86.0% of mail was still evaluated under p=none.")
        );
        r.window.other_policy = EnforcementState::Unknown;
        assert_eq!(
            evidence(&r)[2].hint.as_deref(),
            Some("86.0% of mail was still evaluated under an unknown policy.")
        );
        r.history_days = None;
        r.window.delivered_compliance.pct = None;
        let rows = evidence(&r);
        assert_eq!(rows[1].value, "No reports under p=quarantine");
        assert_eq!(rows[3].value, "Nothing delivered");
        // Enforced and not applicable: no rules to list.
        assert!(evidence(&readiness(ReadinessState::Enforced, 10, None)).is_empty());
    }

    #[test]
    fn the_plan_reads_when_each_step_applies() {
        let plan = vec![
            PolicyTarget { p: Q, pct: 100 },
            PolicyTarget { p: R, pct: 100 },
        ];
        let steps_of = |state| {
            let mut r = readiness(state, 4120, Some(99.8));
            r.plan = plan.clone();
            steps(&r)
                .into_iter()
                .map(|(_, s)| format!("{}: {} ({})", s.when, s.policy, s.primary))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            steps_of(ReadinessState::Ready { next: Q }),
            [
                "Publish now: p=quarantine (true)",
                "Then, once ready again: p=reject (false)"
            ]
        );
        assert_eq!(
            steps_of(not_ready(Q, ReadinessReason::TooLittleMail)),
            [
                "Once ready: p=quarantine (false)",
                "Then, once ready again: p=reject (false)"
            ]
        );
    }
}
