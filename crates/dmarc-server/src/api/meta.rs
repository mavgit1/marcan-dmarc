//! `GET /api/meta`: the rules' fixed numbers, for help text and legends; and
//! what the app frame shows on every page: whether the free-software notice
//! is shown, where the source, the operator license and its billing are,
//! and the newer version the version check found.
//! The stored logo is not here: it is its own `GET /api/settings/branding/logo`,
//! read only where it is shown (the UI carries no operator branding: name and
//! logo are for the emails only).
//!
//! Verdicts never come from here (every response carries its own); this only
//! lets the guide quote the rules without copying them.

use crate::alerts::job::ALERT_INTERVAL;
use crate::api::settings::{LOGO_MAX_BYTES, OPERATOR_NAME_MAX_CHARS};
use crate::client_reports::job::CLIENT_REPORT_NOTICE_DAYS;
use crate::error::AppResult;
use crate::extract::Json;
use crate::license::{Standing, BILLING_URL, LICENSE_URL, SOURCE_URL};
use crate::updates::{newer_version, RUNNING_VERSION};
use crate::wording::alerts::{catalogue_title, catalogue_trigger, group_title};
use crate::wording::issues::{issue_meaning, issue_name};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::{
    severity_points, AlertGroup, IssueKind, Severity, ALERT_CATALOGUE, GRADE_BANDS,
    NEW_SENDER_MIN_FAILURES, RATE_PASS_MIN_PCT, RATE_WARN_MIN_PCT, READINESS_MIN_MESSAGES,
    READINESS_MIN_POLICY_SHARE_PCT, READINESS_WINDOW_DAYS, RSA_GOOD_BITS, RSA_MIN_BITS,
};
use dmarc_store::{
    AppSettings, CLIENT_REPORT_DAY_RANGE, OBSERVED_SELECTOR_WINDOW_SECS,
    REPORT_RETENTION_MONTHS_RANGE,
};
use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Meta {
    /// Marcan's version.
    pub version: String,
    /// A share of good mail passes from this percentage on.
    pub rate_pass_min_pct: f64,
    /// ...warns from this one on, and fails below it.
    pub rate_warn_min_pct: f64,
    /// Days without reports after which a domain counts as having stopped
    /// reporting: the monitoring setting `domain_reports_stopped_days`.
    pub reports_stopped_days: i64,
    /// Policy readiness looks at the last this many days (and needs reports
    /// under the current policy covering all of them).
    pub readiness_window_days: i64,
    /// ...and needs at least this many messages in them under the current policy.
    pub readiness_min_messages: i64,
    /// ...and at least this share of the window's messages evaluated under it.
    pub readiness_min_policy_share_pct: f64,
    /// RSA DKIM keys below this many bits are weak (critical).
    pub rsa_min_bits: usize,
    /// ...and below this many bits dated (warning).
    pub rsa_good_bits: usize,
    /// DKIM selectors count as seen signing when reports show them within
    /// this many days before the domain's newest report.
    pub observed_selector_window_days: i64,
    /// DNS score: points one finding of each severity deducts, worst first.
    pub severity_points: Vec<SeverityPoints>,
    /// DNS grade letters and their lowest score, best first; anything lower is F.
    pub grade_bands: Vec<GradeBand>,
    /// Every kind of alert: what triggers it and its group, in the order
    /// the settings list them. The alert settings mute kinds by `id`.
    pub alert_catalogue: Vec<AlertCatalogueItem>,
    /// Every issue a domain can have open, with its name and what it means
    /// (the open ones come worded with each domain's assessment).
    pub issue_catalogue: Vec<IssueCatalogueItem>,
    /// The catalogue's groups with their headings, in the catalogue's order.
    pub alert_groups: Vec<AlertGroupTitle>,
    /// Every domain is checked for changes this often, right after the DNS
    /// check of that round.
    pub alert_interval_minutes: u64,
    /// A sender new to a domain (nothing from it before the readiness
    /// window) alerts from this many failing messages in the window,
    /// forwarded mail not counted.
    pub new_sender_min_failures: i64,
    /// The organization page says when its client report goes out from
    /// this many days before.
    pub client_report_notice_days: i64,
    /// The largest logo upload, in bytes.
    pub logo_max_bytes: u32,
    /// The longest operator name, in characters.
    pub operator_name_max_chars: u32,
    /// The days of the month a client report can go out on (every month
    /// has them), first and last.
    pub client_report_day_min: u8,
    pub client_report_day_max: u8,
    /// How many months reports can be kept, fewest and most.
    pub report_retention_months_min: u32,
    pub report_retention_months_max: u32,
    /// Where the source code of the version this server runs is.
    pub source_url: String,
    /// The license page of the docs (the terms, the key and the notice).
    pub license_url: String,
    /// Where the subscription behind an operator key is managed or cancelled.
    pub billing_url: String,
    /// Show the notice: unless a valid operator key is stored. Changes with
    /// the license settings.
    pub notice_shown: bool,
    /// The email of the stored operator key, when it is valid.
    pub licensed_to: Option<String>,
    /// A newer version than the running one, as the version check last
    /// named it (the sidebar and Settings → License say so); `null` while
    /// the check is off, before its first answer, and when the running
    /// version is the newest.
    pub available: Option<AvailableVersion>,
}

/// A newer release the version check found.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AvailableVersion {
    /// "0.3.1".
    pub version: String,
    /// Its release notes, on the docs site.
    pub url: String,
}

/// One kind of alert as the user can choose it. `id` is stable (stored in
/// the alert settings' muted kinds); `title` names it, `trigger` says in one
/// sentence what makes it.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AlertCatalogueItem {
    pub id: &'static str,
    pub group: AlertGroup,
    pub title: String,
    pub trigger: String,
}

/// One kind of issue: `name` as sentences say it ("delivered failures"),
/// `meaning` when it is raised.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct IssueCatalogueItem {
    pub kind: IssueKind,
    pub name: &'static str,
    pub meaning: String,
}

/// A group of the alert catalogue and its heading.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AlertGroupTitle {
    pub group: AlertGroup,
    pub title: &'static str,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SeverityPoints {
    pub severity: Severity,
    pub points: u8,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct GradeBand {
    pub grade: String,
    pub min_score: u8,
}

/// Each group of [`ALERT_CATALOGUE`] once, in the order its entries come.
fn alert_groups() -> Vec<AlertGroupTitle> {
    let mut groups: Vec<AlertGroupTitle> = Vec::new();
    for entry in &ALERT_CATALOGUE {
        let group = entry.group();
        if groups.iter().all(|g| g.group != group) {
            groups.push(AlertGroupTitle {
                group,
                title: group_title(group),
            });
        }
    }
    groups
}

pub async fn get_meta(State(state): State<Arc<AppState>>) -> AppResult<Json<Meta>> {
    let settings = AppSettings::load(&state.pool).await?;
    let standing = Standing::new(&settings.license);
    let update_check = dmarc_store::get_update_check(&state.pool).await?;
    let available =
        newer_version(settings.updates, update_check.as_ref(), RUNNING_VERSION).map(|seen| {
            AvailableVersion {
                version: seen.version.clone(),
                url: seen.url.clone(),
            }
        });
    Ok(Json(Meta {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        rate_pass_min_pct: RATE_PASS_MIN_PCT,
        rate_warn_min_pct: RATE_WARN_MIN_PCT,
        reports_stopped_days: i64::from(settings.monitoring.domain_reports_stopped_days),
        readiness_window_days: READINESS_WINDOW_DAYS,
        readiness_min_messages: READINESS_MIN_MESSAGES,
        readiness_min_policy_share_pct: READINESS_MIN_POLICY_SHARE_PCT,
        rsa_min_bits: RSA_MIN_BITS,
        rsa_good_bits: RSA_GOOD_BITS,
        observed_selector_window_days: OBSERVED_SELECTOR_WINDOW_SECS / 86_400,
        severity_points: Severity::ALL
            .into_iter()
            .map(|severity| SeverityPoints {
                severity,
                points: severity_points(severity),
            })
            .collect(),
        grade_bands: GRADE_BANDS
            .iter()
            .map(|(grade, min_score)| GradeBand {
                grade: (*grade).to_owned(),
                min_score: *min_score,
            })
            .collect(),
        alert_catalogue: ALERT_CATALOGUE
            .iter()
            .map(|&kind| AlertCatalogueItem {
                id: kind.id(),
                group: kind.group(),
                title: catalogue_title(kind),
                trigger: catalogue_trigger(kind),
            })
            .collect(),
        issue_catalogue: IssueKind::ALL
            .into_iter()
            .map(|kind| IssueCatalogueItem {
                kind,
                name: issue_name(kind),
                meaning: issue_meaning(kind),
            })
            .collect(),
        alert_groups: alert_groups(),
        alert_interval_minutes: ALERT_INTERVAL.as_secs() / 60,
        new_sender_min_failures: NEW_SENDER_MIN_FAILURES,
        client_report_notice_days: CLIENT_REPORT_NOTICE_DAYS,
        logo_max_bytes: LOGO_MAX_BYTES as u32,
        operator_name_max_chars: OPERATOR_NAME_MAX_CHARS as u32,
        client_report_day_min: CLIENT_REPORT_DAY_RANGE.0,
        client_report_day_max: CLIENT_REPORT_DAY_RANGE.1,
        report_retention_months_min: REPORT_RETENTION_MONTHS_RANGE.0,
        report_retention_months_max: REPORT_RETENTION_MONTHS_RANGE.1,
        source_url: SOURCE_URL.to_owned(),
        license_url: LICENSE_URL.to_owned(),
        billing_url: BILLING_URL.to_owned(),
        notice_shown: standing.notice_shown,
        licensed_to: standing.licensed_to,
        available,
    }))
}
