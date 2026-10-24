//! Each domain's plan (its platform, mailing services, forwarders and
//! spoofers, its policy over time) and the aggregate reports its mail
//! produces, day by day, as receivers write them.

use crate::catalogue::{IdStyle, Reporter, GENERIC_REPORTERS, TAIL_REPORTERS, TAIL_SPLIT};
use crate::profile::{DnsClass, Profile, Shape, DAYS};
use crate::rng::Rng;
use crate::services::Services;
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const DAY: i64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Microsoft,
    Google,
    Hoster(usize),
}

/// What kind of domain a plan is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Shaped,
    Clean,
    Parked,
    Ignored,
}

pub struct Plan {
    pub name: &'static str,
    pub org: usize,
    pub role: Role,
    pub shape: Shape,
    pub platform: Platform,
    /// The selectors the platform signs with; for
    /// [`DnsClass::NoneMissingSelector`] the last one is not published.
    pub selectors: Vec<&'static str>,
    /// Mailing services that sign for the domain, with their addresses.
    pub esps: Vec<(usize, Vec<String>)>,
    /// A mailing service never set up for the domain: its mail fails.
    pub broken_esp: Option<(usize, Vec<String>)>,
    pub platform_ips: Vec<String>,
    pub forward_ips: Vec<String>,
    pub spoof_ips: Vec<String>,
    /// The policy before a change `days ago` (the DNS shows the new one).
    pub policy_before: Option<(i64, &'static str)>,
    /// Reports stopped this many days ago.
    pub stopped_ago: Option<i64>,
    /// Per [`reporters`] entry.
    pub reporter_weights: Vec<f64>,
}

impl Plan {
    /// `p=` on the day `days_ago` days before today.
    pub fn policy_on(&self, days_ago: i64) -> &'static str {
        match self.policy_before {
            Some((changed, before)) if days_ago > changed => before,
            _ => self.shape.dns.policy(),
        }
    }

    pub fn active_on(&self, days_ago: i64) -> bool {
        self.role != Role::Parked
            && self.role != Role::Ignored
            && self.shape.messages > 0.0
            && days_ago <= self.shape.first_ago
            && self.stopped_ago.is_none_or(|s| days_ago > s)
    }
}

/// Every receiver: the generic ones, then the tail.
pub fn reporters() -> Vec<Reporter> {
    GENERIC_REPORTERS
        .iter()
        .chain(TAIL_REPORTERS.iter())
        .copied()
        .collect()
}

/// The shape of a domain in a clean state, from its rank among them.
pub fn clean_shape(rank: usize) -> Shape {
    let messages = [2400.0, 1300.0, 820.0, 380.0][rank % 4];
    Shape {
        messages,
        passed: 0.994,
        blocked: 0.006,
        forwarded: 0.0,
        delivered: 0.0,
        ips: [38, 24, 15, 9][rank % 4],
        first_ago: DAYS,
        dns: DnsClass::Clean,
    }
}

pub fn parked_shape() -> Shape {
    Shape {
        messages: 0.0,
        passed: 0.0,
        blocked: 0.0,
        forwarded: 0.0,
        delivered: 0.0,
        ips: 0,
        first_ago: 0,
        dns: DnsClass::Clean,
    }
}

/// Decide who sends a domain's mail.
pub fn plan(
    rng: &mut Rng,
    services: &Services,
    profile: &Profile,
    name: &'static str,
    org: usize,
    role: Role,
    shape: Shape,
) -> Plan {
    let n = shape.ips.max(1) as f64;
    let failures = shape.blocked + shape.delivered;
    let weights = [shape.passed, shape.forwarded * 4.0, failures * 4.0];
    let total: f64 = weights.iter().sum::<f64>().max(1e-9);
    let part = |share: f64, w: f64| {
        if share > 0.0 {
            ((n * w / total).round() as usize).max(1)
        } else {
            0
        }
    };
    let n_pass = part(shape.passed, weights[0]);
    let n_fwd = part(shape.forwarded, weights[1]);
    let n_fail = part(failures, weights[2]);

    // Mailing services: bigger senders use one or two.
    let mut esp_ids: Vec<usize> = (0..services.esps.len()).collect();
    rng.shuffle(&mut esp_ids);
    let esp_count = if shape.messages > 150.0 && rng.chance(0.55) {
        1 + usize::from(rng.chance(0.35))
    } else {
        usize::from(role == Role::Clean)
    };
    let mut esps = Vec::new();
    let mut esp_ips = 0;
    for &e in esp_ids.iter().take(esp_count) {
        let k = (1 + rng.below(4)).min((n_pass / 3).max(1));
        esps.push((
            e,
            rng.sample(&services.esps[e].hosts, k)
                .into_iter()
                .map(|h| h.ip)
                .collect(),
        ));
        esp_ips += k;
    }
    // A service sending unaligned mail explains a large blocked share or
    // part of the delivered failures.
    let broken_esp = if shape.blocked > 0.2 || (shape.delivered > 0.02 && rng.chance(0.5)) {
        let e = esp_ids[esp_count % esp_ids.len()];
        let k = 1 + rng.below(2);
        Some((
            e,
            rng.sample(&services.esps[e].hosts, k)
                .into_iter()
                .map(|h| h.ip)
                .collect(),
        ))
    } else {
        None
    };

    let n_platform = n_pass.saturating_sub(esp_ips).max(usize::from(n_pass > 0));
    let platform = if n_platform > 40 {
        Platform::Microsoft
    } else {
        match rng.weighted(&[0.62, 0.26, 0.12]) {
            0 => Platform::Microsoft,
            1 => Platform::Google,
            _ => Platform::Hoster(rng.below(services.hosters.len())),
        }
    };
    let (pool, selectors): (&[crate::services::Host], Vec<&'static str>) = match platform {
        Platform::Microsoft => (&services.microsoft, vec!["selector1", "selector2"]),
        Platform::Google => (&services.google, vec!["google", "google2024"]),
        Platform::Hoster(h) => (
            &services.hosters[h].hosts,
            vec![services.hosters[h].selector, "mail2023"],
        ),
    };
    let platform_ips = rng
        .sample(pool, n_platform)
        .into_iter()
        .map(|h| h.ip)
        .collect();

    // Forwarders: a few gateways, other Microsoft tenants, Google.
    let mut candidates: Vec<String> = Vec::new();
    let mut forwarder_ids: Vec<usize> = (0..services.forwarders.len()).collect();
    rng.shuffle(&mut forwarder_ids);
    for &f in forwarder_ids.iter().take(1 + rng.below(3)) {
        candidates.extend(services.forwarders[f].hosts.iter().map(|h| h.ip.clone()));
    }
    candidates.extend(
        rng.sample(&services.microsoft, n_fwd)
            .into_iter()
            .map(|h| h.ip),
    );
    let forward_ips = rng.sample(&candidates, n_fwd);
    let spoof_ips = rng
        .sample(&services.spoofers, n_fail)
        .into_iter()
        .map(|h| h.ip)
        .collect();

    // Receivers: everyone's share, jittered per domain; a domain hears from
    // a few of the tail only.
    let mut reporter_weights = Vec::new();
    let generic = &profile.reporters[..GENERIC_REPORTERS.len()];
    for share in generic {
        reporter_weights.push(share * rng.range(0.5, 1.5));
    }
    let tail = profile.reporters[GENERIC_REPORTERS.len()];
    for split in TAIL_SPLIT {
        let heard = rng.chance(0.45);
        reporter_weights.push(if heard {
            tail * split * rng.range(1.0, 3.0)
        } else {
            0.0
        });
    }

    Plan {
        name,
        org,
        role,
        shape,
        platform,
        selectors,
        esps,
        broken_esp,
        platform_ips,
        forward_ips,
        spoof_ips,
        policy_before: None,
        stopped_ago: None,
        reporter_weights,
    }
}

/// What a report row is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Platform(usize),
    Esp(usize),
    BrokenEsp(usize),
    Forward,
    Spoof,
}

/// Outcomes, as the alerts count them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    Passed,
    Blocked,
    Forwarded,
    Delivered,
}

/// A domain's traffic: its reports and the day-by-day figures behind them.
pub struct Traffic {
    pub xml: Vec<String>,
    /// Per day (oldest first): passed, blocked, forwarded, delivered.
    pub daily: Vec<[i64; 4]>,
    /// Failing mail per sender: `(day, ip, delivered, blocked)`.
    pub failing: Vec<(usize, String, i64, i64)>,
    pub ips: Vec<String>,
}

/// Generate a domain's reports for the [`DAYS`] ending yesterday (`today`
/// is midnight UTC).
pub fn traffic(
    rng: &mut Rng,
    services: &Services,
    profile: &Profile,
    plan: &Plan,
    today: i64,
) -> Traffic {
    let reporters = reporters();
    let weight = |days_ago: i64| {
        let begin = today - days_ago * DAY;
        // 1970-01-01 was a Thursday: (day + 3) % 7 is 0 on Mondays.
        if (begin / DAY + 3) % 7 >= 5 {
            profile.weekend
        } else {
            1.0
        }
    };
    let active_weight: f64 = (1..=DAYS)
        .filter(|&a| {
            plan.active_on(a)
                || plan
                    .stopped_ago
                    .is_some_and(|s| a <= s && a <= plan.shape.first_ago)
        })
        .map(weight)
        .sum();
    let base = if active_weight > 0.0 {
        plan.shape.messages / active_weight
    } else {
        0.0
    };
    let mpr = profile.messages_per_row;
    let mut out = Traffic {
        xml: Vec::new(),
        daily: vec![[0; 4]; DAYS as usize],
        failing: Vec::new(),
        ips: Vec::new(),
    };
    let mut seen = std::collections::BTreeSet::new();

    for days_ago in (1..=DAYS).rev() {
        if !plan.active_on(days_ago) {
            continue;
        }
        let day = (DAYS - days_ago) as usize;
        let policy = plan.policy_on(days_ago);
        let volume = base * weight(days_ago) * rng.range(0.7, 1.3);
        let s = &plan.shape;
        let (mut blocked, mut delivered) = (s.blocked, s.delivered);
        if policy == "none" {
            delivered += blocked;
            blocked = 0.0;
        }
        let mut draw = |share: f64, spread: f64| {
            let jitter = rng.range(1.0 - spread, 1.0 + spread);
            rng.round(volume * share * jitter)
        };
        let counts = [
            (Outcome::Passed, draw(s.passed, 0.08)),
            (Outcome::Blocked, draw(blocked, 0.5)),
            (Outcome::Forwarded, draw(s.forwarded, 0.5)),
            (Outcome::Delivered, draw(delivered, 0.5)),
        ];

        // Rows per receiver, merged by address and kind.
        let mut by_reporter: BTreeMap<usize, BTreeMap<(String, Kind, Outcome), i64>> =
            BTreeMap::new();
        for (outcome, mut left) in counts {
            while left > 0 {
                let (kind, ip, size) = match row_source(rng, plan, outcome) {
                    Some(source) => source,
                    None => break,
                };
                let mean = match kind {
                    Kind::Esp(_) | Kind::BrokenEsp(_) => mpr * 4.0,
                    Kind::Spoof => 1.5,
                    _ => mpr,
                } * size;
                let count = (1 + geometric(rng, mean - 1.0)).min(left);
                left -= count;
                out.daily[day][outcome as usize] += count;
                let reporter = rng.weighted(&plan.reporter_weights);
                if matches!(kind, Kind::Spoof | Kind::BrokenEsp(_)) {
                    let (d, b) = if outcome == Outcome::Blocked {
                        (0, count)
                    } else {
                        (count, 0)
                    };
                    out.failing.push((day, ip.clone(), d, b));
                }
                if seen.insert(ip.clone()) {
                    out.ips.push(ip.clone());
                }
                *by_reporter
                    .entry(reporter)
                    .or_default()
                    .entry((ip, kind, outcome))
                    .or_default() += count;
            }
        }

        let begin = today - days_ago * DAY;
        for (r, rows) in by_reporter {
            let reporter = reporters[r];
            let mut records = String::new();
            for ((ip, kind, outcome), count) in rows {
                record_xml(
                    &mut records,
                    services,
                    plan,
                    reporter,
                    &ip,
                    kind,
                    outcome,
                    policy,
                    count,
                );
            }
            out.xml
                .push(report_xml(rng, reporter, plan, policy, begin, &records));
        }
    }
    out
}

/// Where a row of `outcome` comes from: its kind, address and a size factor.
fn row_source(rng: &mut Rng, plan: &Plan, outcome: Outcome) -> Option<(Kind, String, f64)> {
    match outcome {
        Outcome::Passed => {
            if !plan.esps.is_empty() && rng.chance(0.22) {
                let (e, ips) = rng.pick(&plan.esps);
                return Some((Kind::Esp(*e), rng.pick(ips).clone(), 1.0));
            }
            if plan.platform_ips.is_empty() {
                return None;
            }
            // Microsoft 365 alternates its two selectors.
            let selector = usize::from(plan.selectors.len() > 1 && rng.chance(0.3));
            Some((
                Kind::Platform(selector),
                rng.pick(&plan.platform_ips).clone(),
                1.0,
            ))
        }
        Outcome::Forwarded => {
            let ips = if plan.forward_ips.is_empty() {
                &plan.platform_ips
            } else {
                &plan.forward_ips
            };
            (!ips.is_empty()).then(|| (Kind::Forward, rng.pick(ips).clone(), 1.0))
        }
        Outcome::Blocked | Outcome::Delivered => {
            let broken_share = if outcome == Outcome::Blocked {
                0.7
            } else {
                0.4
            };
            if let Some((e, ips)) = &plan.broken_esp {
                if rng.chance(broken_share) || plan.spoof_ips.is_empty() {
                    return Some((Kind::BrokenEsp(*e), rng.pick(ips).clone(), 1.0));
                }
            }
            (!plan.spoof_ips.is_empty())
                .then(|| (Kind::Spoof, rng.pick(&plan.spoof_ips).clone(), 1.0))
        }
    }
}

/// A geometric draw with the given mean (0 for a mean at or below 0).
fn geometric(rng: &mut Rng, mean: f64) -> i64 {
    if mean <= 0.0 {
        return 0;
    }
    let p = 1.0 / (1.0 + mean);
    let u = rng.f64().max(1e-12);
    (u.ln() / (1.0 - p).ln()).floor() as i64
}

/// SPF's result for mail from an address the domain does not list.
fn unlisted_spf(dns: DnsClass) -> &'static str {
    match dns {
        DnsClass::RejectSoftfail | DnsClass::NoneOverLimit => "softfail",
        DnsClass::NoneNeutral => "neutral",
        _ => "fail",
    }
}

/// `(domain, result)`, or the evaluated `(dkim, spf)`.
type Pair<'a> = (&'a str, &'a str);
/// A DKIM signature: `(domain, selector, result)`.
type Signature<'a> = (&'a str, &'a str, &'a str);

#[allow(clippy::too_many_arguments)]
fn record_xml(
    out: &mut String,
    services: &Services,
    plan: &Plan,
    reporter: Reporter,
    ip: &str,
    kind: Kind,
    outcome: Outcome,
    policy: &str,
    count: i64,
) {
    let domain = plan.name;
    let disposition = match outcome {
        Outcome::Blocked => policy,
        _ => "none",
    };
    let mut reason = None;
    let (envelope, spf, dkim, eval): (&str, Pair, Vec<Signature>, Pair) = match kind {
        Kind::Platform(s) => (
            domain,
            (domain, "pass"),
            vec![(domain, plan.selectors[s], "pass")],
            ("pass", "pass"),
        ),
        Kind::Esp(e) => {
            let esp = &services.esps[e];
            (
                esp.envelope,
                (esp.envelope, "pass"),
                vec![(domain, esp.selector, "pass"), (esp.signer, "s1", "pass")],
                ("pass", "fail"),
            )
        }
        Kind::BrokenEsp(e) => {
            let esp = &services.esps[e];
            (
                esp.envelope,
                (esp.envelope, "pass"),
                vec![(esp.signer, "s1", "pass")],
                ("fail", "fail"),
            )
        }
        Kind::Forward => {
            // Microsoft names a verified ARC chain, the others the forward.
            reason = Some(if reporter.org.contains("Outlook") {
                ("local_policy", "arc=pass")
            } else {
                ("forwarded", "")
            });
            (
                domain,
                (domain, unlisted_spf(plan.shape.dns)),
                vec![(domain, plan.selectors[0], "fail")],
                ("fail", "fail"),
            )
        }
        Kind::Spoof => (
            domain,
            (domain, unlisted_spf(plan.shape.dns)),
            Vec::new(),
            ("fail", "fail"),
        ),
    };
    if outcome == Outcome::Delivered && policy != "none" {
        reason = Some(("local_policy", ""));
    }
    let _ = write!(
        out,
        "  <record>\n    <row>\n      <source_ip>{ip}</source_ip>\n      <count>{count}</count>\n      <policy_evaluated>\n        <disposition>{disposition}</disposition>\n        <dkim>{}</dkim>\n        <spf>{}</spf>\n",
        eval.0, eval.1
    );
    if let Some((kind, comment)) = reason {
        let _ = writeln!(
            out,
            "        <reason><type>{kind}</type><comment>{comment}</comment></reason>"
        );
    }
    let _ = write!(
        out,
        "      </policy_evaluated>\n    </row>\n    <identifiers>\n      <envelope_from>{envelope}</envelope_from>\n      <header_from>{domain}</header_from>\n    </identifiers>\n    <auth_results>\n"
    );
    for (d, selector, result) in dkim {
        let _ = writeln!(
            out,
            "      <dkim><domain>{d}</domain><selector>{selector}</selector><result>{result}</result></dkim>"
        );
    }
    let _ = write!(
        out,
        "      <spf><domain>{}</domain><scope>mfrom</scope><result>{}</result></spf>\n    </auth_results>\n  </record>\n",
        spf.0, spf.1
    );
}

fn report_id(rng: &mut Rng, style: IdStyle, begin: i64) -> String {
    match style {
        IdStyle::Digits => rng.digits(20),
        IdStyle::Hex => rng.hex(32),
        IdStyle::Stamped => format!("{begin}.{}", rng.digits(6)),
        IdStyle::Uuid => format!(
            "{}-{}-{}-{}-{}",
            rng.hex(8),
            rng.hex(4),
            rng.hex(4),
            rng.hex(4),
            rng.hex(12)
        ),
    }
}

fn report_xml(
    rng: &mut Rng,
    reporter: Reporter,
    plan: &Plan,
    policy: &str,
    begin: i64,
    records: &str,
) -> String {
    let id = report_id(rng, reporter.id, begin);
    let end = begin + DAY - 1;
    let (org, email, domain) = (reporter.org, reporter.email, plan.name);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feedback>\n  <version>1.0</version>\n  <report_metadata>\n    <org_name>{org}</org_name>\n    <email>{email}</email>\n    <report_id>{id}</report_id>\n    <date_range><begin>{begin}</begin><end>{end}</end></date_range>\n  </report_metadata>\n  <policy_published>\n    <domain>{domain}</domain>\n    <adkim>r</adkim>\n    <aspf>r</aspf>\n    <p>{policy}</p>\n    <sp>{policy}</sp>\n    <pct>100</pct>\n  </policy_published>\n{records}</feedback>\n"
    )
}
