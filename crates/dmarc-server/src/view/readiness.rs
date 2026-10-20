//! A domain's policy readiness as the domain page shows it: the verdict
//! (`dmarc_assess::PolicyReadiness`) with its words from
//! [`crate::wording::readiness`], so the UI only colours and lays them out.

use crate::wording::readiness as words;
use dmarc_assess::{PolicyReadiness, PolicyStep};
use dmarc_dns::dmarc_publish::with_policy;
use serde::Serialize;
use ts_rs::TS;

/// Readiness for the next policy step, judged and worded.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Readiness {
    #[serde(flatten)]
    #[ts(flatten)]
    pub judged: PolicyReadiness,
    /// The verdict ("Ready for p=quarantine"), coloured by `status`.
    pub lead: String,
    /// Why, read after the lead and a colon.
    pub detail: String,
    /// The verdict in a few words under the domain's title: the lead;
    /// `null` where the badges say it (no policy known, parked).
    pub headline: Option<String>,
    /// The rules of the next step with what the window showed, in the order
    /// the verdict reads them; empty when they do not decide the state
    /// (enforced, nothing to judge).
    pub evidence: Vec<EvidenceRow>,
    /// The records to publish from here (`plan`), the next first, each
    /// written into the domain's record.
    pub steps: Vec<PlanStep>,
    /// Under "Ready for p=reject" of a quarantining domain: the mail the
    /// policy already blocked in the window, refused once the step is taken;
    /// `null` otherwise or with none.
    pub blocked_note: Option<String>,
    /// A parked domain, with a DMARC record or without: its own verdict and
    /// the window's mail; `null` for any other domain.
    pub parked: Option<ParkedReadiness>,
}

impl Readiness {
    /// `judged` with its words; `parked`: the domain is marked parked;
    /// `record`: the DMARC record the steps are written into (the published
    /// one, else a new one).
    pub fn new(judged: PolicyReadiness, parked: bool, record: &str) -> Self {
        let verdict = words::verdict(&judged);
        let steps = words::steps(&judged)
            .into_iter()
            .map(|(step, words)| PlanStep {
                record: with_policy(record, step_tag(step.p), step.pct),
                p: step.p,
                pct: step.pct,
                policy: words.policy,
                when: words.when,
                primary: words.primary,
            })
            .collect();
        Self {
            headline: words::headline(&judged),
            evidence: words::evidence(&judged),
            steps,
            blocked_note: words::blocked_note(&judged),
            parked: parked.then(|| words::parked(&judged)).flatten(),
            lead: verdict.lead,
            detail: verdict.detail,
            judged,
        }
    }
}

/// One rule of the next step: `value` is what the window showed, `rule`
/// what the step needs (`null` when the value says it), `met` the server's
/// answer; read together as "value, rule" ("140 under p=quarantine, needs
/// 200"). `hint` says what holds an unmet rule back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EvidenceRow {
    pub label: String,
    pub value: String,
    pub rule: Option<String>,
    pub met: bool,
    pub hint: Option<String>,
}

/// The `p=` value of a step.
fn step_tag(step: PolicyStep) -> &'static str {
    match step {
        PolicyStep::Quarantine => "quarantine",
        PolicyStep::Reject => "reject",
    }
}

/// One policy to publish: `p` at `pct`, named (`policy`: "p=quarantine"),
/// when it applies (`when`: "Publish now", "Once ready", "Then, once ready
/// again"), whether it is the record the verdict recommends now (`primary`;
/// not ready, none is), and the record itself (`record`: the domain's with
/// `p` and `pct` changed, every other tag as published).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PlanStep {
    pub p: PolicyStep,
    pub pct: i64,
    pub policy: String,
    pub when: String,
    pub primary: bool,
    pub record: String,
}

/// A parked domain's verdict, read as "lead detail" ("Publish p=reject
/// now." and why, or "Enforced:" and nothing to do), and the line over the
/// window's mail, every message of which a parked domain should not send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ParkedReadiness {
    pub lead: String,
    pub detail: String,
    pub mail_line: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wording::readiness::tests::readiness;
    use dmarc_assess::{PolicyTarget, ReadinessState};

    /// Each step is the domain's record with its policy, every other tag
    /// as published, and reads back as that policy.
    #[test]
    fn each_step_is_written_into_the_record() {
        let mut judged = readiness(
            ReadinessState::Ready {
                next: PolicyStep::Quarantine,
            },
            4_000,
            Some(99.8),
        );
        judged.plan = vec![
            PolicyTarget {
                p: PolicyStep::Quarantine,
                pct: 25,
            },
            PolicyTarget {
                p: PolicyStep::Reject,
                pct: 100,
            },
        ];
        let page = Readiness::new(
            judged,
            false,
            "v=DMARC1; p=none; sp=none; rua=mailto:d@example.com",
        );
        let records: Vec<&str> = page.steps.iter().map(|s| s.record.as_str()).collect();
        assert_eq!(
            records,
            [
                "v=DMARC1; p=quarantine; sp=none; rua=mailto:d@example.com; pct=25",
                "v=DMARC1; p=reject; sp=none; rua=mailto:d@example.com",
            ]
        );
        let parsed = dmarc_dns::dmarc::parse_dmarc_record(records[0]);
        assert_eq!(
            (parsed.p.as_deref(), parsed.pct, parsed.sp.as_deref()),
            (Some("quarantine"), Some(25), Some("none"))
        );
    }
}
