//! `app_settings`: one row per group of Marcan's settings (mailbox, reporting,
//! monitoring, retention, DNS, sending, alerts, client reports, license,
//! branding, onboarding), keyed by the group's name and holding the group as
//! JSON, read together as one typed [`AppSettings`] and written a group at a
//! time with [`save_settings`]. A field missing from a stored group reads as
//! its default. The same table keeps a few job records as plain text by key
//! ([`get_setting`], [`set_setting`]); the logo (`branding_logo`) and the extra
//! DKIM selectors (`dkim_selectors`) have tables of their own.

mod alerts;
mod branding;
mod client_reports;
mod dns;
mod license;
mod mailbox;
mod monitoring;
mod onboarding;
mod reporting;
mod retention;
mod sending;

pub use alerts::{
    get_alerts_held_at, get_last_digest_at, set_alerts_held_at, set_last_digest_at, AlertSettings,
    DigestFrequency, Weekday, DEFAULT_DIGEST_HOUR,
};
pub use branding::{
    get_logo, get_logo_info, set_logo, BrandingSettings, LogoInfo, LogoType, PixelSize, StoredLogo,
};
pub use client_reports::{
    get_client_reports_held_at, set_client_reports_held_at, ClientReportSettings,
    CLIENT_REPORT_DAY_RANGE, DEFAULT_CLIENT_REPORT_DAY, DEFAULT_CLIENT_REPORT_HOUR,
};
pub use dns::{
    get_custom_dkim_selectors, get_demo_dns_records, set_custom_dkim_selectors,
    set_demo_dns_records, DnsSettings,
};
pub use license::LicenseSettings;
pub use mailbox::{ImapSecurity, MailboxSettings};
pub use monitoring::{
    MonitoringSettings, DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS,
    DEFAULT_REPORT_FRESHNESS_WARNING_HOURS, DOMAIN_REPORTS_STOPPED_DAYS_RANGE,
    REPORT_FRESHNESS_WARNING_HOURS_RANGE,
};
pub use onboarding::{OnboardingSettings, OnboardingState};
pub use reporting::{ReportingMode, ReportingSettings};
pub use retention::{
    RetentionSettings, DEFAULT_REPORT_RETENTION_MONTHS, REPORT_RETENTION_MONTHS_RANGE,
};
pub use sending::{SendingGroup, SendingLogin, SendingSettings, SmtpSecurity};

use crate::StoreError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use std::collections::HashMap;

/// One group of settings: its row in `app_settings` and its JSON, the shape
/// the API carries it in where it has one.
pub trait SettingsGroup: Serialize + DeserializeOwned + Default {
    /// The group's key in `app_settings`.
    const KEY: &'static str;
}

/// A stored group read over its default, field by field: a field it did not
/// have when it was stored reads as the default. (The group types themselves
/// stay strict, so a `PUT` without a field is still refused.)
fn read_group<G: SettingsGroup>(raw: &str) -> Result<G, serde_json::Error> {
    let mut value = serde_json::to_value(G::default())?;
    match (&mut value, serde_json::from_str(raw)?) {
        (serde_json::Value::Object(base), serde_json::Value::Object(stored)) => base.extend(stored),
        (_, stored) => value = stored,
    }
    serde_json::from_value(value)
}

/// Write one group as a whole (one row, so a reader never sees half of it).
pub async fn save_settings<G: SettingsGroup>(
    pool: &SqlitePool,
    group: &G,
) -> Result<(), StoreError> {
    set_setting(pool, G::KEY, &serde_json::to_string(group)?).await
}

/// One group as stored, its default when none is; an error when the stored
/// value does not read as the group (for a caller that must not act on a
/// default in its place, such as the retention clean-up).
pub async fn load_settings<G: SettingsGroup>(pool: &SqlitePool) -> Result<G, StoreError> {
    match get_setting(pool, G::KEY).await? {
        Some(raw) => Ok(read_group(&raw)?),
        None => Ok(G::default()),
    }
}

/// A raw `app_settings` value by key: the job records, kept beside the
/// settings they follow.
pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// Store or replace a raw `app_settings` value.
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// A job record stored as unix seconds; `None` when not stored.
async fn get_time(pool: &SqlitePool, key: &str) -> Result<Option<i64>, StoreError> {
    Ok(get_setting(pool, key)
        .await?
        .and_then(|v| v.trim().parse().ok()))
}

async fn set_time(pool: &SqlitePool, key: &str, at: i64) -> Result<(), StoreError> {
    set_setting(pool, key, &at.to_string()).await
}

/// Marcan's own settings, by group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppSettings {
    pub mailbox: MailboxSettings,
    pub reporting: ReportingSettings,
    pub monitoring: MonitoringSettings,
    pub retention: RetentionSettings,
    pub dns: DnsSettings,
    pub sending: SendingSettings,
    pub alerts: AlertSettings,
    pub client_reports: ClientReportSettings,
    pub license: LicenseSettings,
    pub branding: BrandingSettings,
    pub onboarding: OnboardingSettings,
}

impl AppSettings {
    /// Every group in one query. A stored group that does not read is its
    /// default (and logged).
    pub async fn load(pool: &SqlitePool) -> Result<Self, StoreError> {
        let rows: HashMap<String, String> =
            sqlx::query_as::<_, (String, String)>("SELECT key, value FROM app_settings")
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect();
        fn group<G: SettingsGroup>(rows: &HashMap<String, String>) -> G {
            let Some(raw) = rows.get(G::KEY) else {
                return G::default();
            };
            read_group(raw).unwrap_or_else(|e| {
                tracing::warn!(group = G::KEY, error = %e, "stored settings do not read; using the defaults");
                G::default()
            })
        }
        Ok(Self {
            mailbox: group(&rows),
            reporting: group(&rows),
            monitoring: group(&rows),
            retention: group(&rows),
            dns: group(&rows),
            sending: group(&rows),
            alerts: group(&rows),
            client_reports: group(&rows),
            license: group(&rows),
            branding: group(&rows),
            onboarding: group(&rows),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `group` written and read back as it is stored.
    fn round_trip<G: SettingsGroup + std::fmt::Debug + PartialEq>(group: &G) {
        let text = serde_json::to_string(group).unwrap();
        assert_eq!(&read_group::<G>(&text).unwrap(), group, "{text}");
    }

    /// `json` read as the stored group `G`.
    fn read<G: SettingsGroup>(json: &serde_json::Value) -> G {
        read_group(&json.to_string()).unwrap()
    }

    #[test]
    fn every_group_reads_back_what_it_wrote() {
        round_trip(&MailboxSettings {
            host: "imap.example.com".into(),
            port: Some(143),
            security: Some(ImapSecurity::StartTls),
            username: "reports@example.com".into(),
            password: "  spaced secret  ".into(),
            folder: Some("DMARC".into()),
            poll_interval_secs: Some(120),
        });
        round_trip(&ReportingSettings {
            report_address: Some("dmarc@example.com".into()),
            reporting_mode: Some(ReportingMode::Forwarded),
        });
        round_trip(&MonitoringSettings {
            report_freshness_warning_hours: 48,
            domain_reports_stopped_days: 10,
        });
        round_trip(&RetentionSettings {
            report_retention_months: 24,
        });
        round_trip(&DnsSettings {
            nameservers: vec!["9.9.9.9".into(), "2001:db8::1".into()],
        });
        round_trip(&SendingSettings {
            group: SendingGroup {
                enabled: true,
                host: "smtp.example.com".into(),
                port: Some(587),
                security: SmtpSecurity::StartTls,
                same_login_as_mailbox: false,
                username: "alerts@example.com".into(),
                sender: "dmarc@example.com".into(),
                reply_to: "help@example.com".into(),
                test_recipient: "me@example.com".into(),
            },
            password: "  spaced secret  ".into(),
        });
        round_trip(&AlertSettings {
            enabled: true,
            recipients: vec!["a@example.com".into(), "b@example.com".into()],
            digest: DigestFrequency::Weekly,
            digest_hour: 18,
            digest_weekday: Weekday::Friday,
            base_url: "https://marcan.example.com".into(),
            muted_kinds: vec!["dns_record_changed.bimi".into(), "health_changed".into()],
        });
        round_trip(&ClientReportSettings {
            enabled: true,
            day_of_month: 15,
            hour_utc: 18,
            reply_to: "support@msp.example".into(),
        });
        round_trip(&LicenseSettings {
            operator_key: Some("you@company.example.AbC".into()),
        });
        round_trip(&BrandingSettings {
            operator_name: "Acme IT".into(),
        });
        round_trip(&OnboardingSettings {
            state: Some(OnboardingState::Skipped),
        });
    }

    #[test]
    fn a_missing_field_reads_as_its_default() {
        assert_eq!(
            read::<SendingSettings>(&json!({})),
            SendingSettings::default()
        );
        let sending: SendingSettings = read(&json!({ "enabled": true, "password": "p" }));
        assert!(sending.group.enabled && sending.group.same_login_as_mailbox);
        assert_eq!(sending.password, "p");
        let monitoring: MonitoringSettings = read(&json!({ "report_freshness_warning_hours": 48 }));
        assert_eq!(
            monitoring.domain_reports_stopped_days,
            DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS
        );
        let alerts: AlertSettings = read(&json!({ "enabled": true }));
        assert_eq!(alerts.digest_hour, DEFAULT_DIGEST_HOUR);
        let reports: ClientReportSettings = read(&json!({ "hour_utc": 9 }));
        assert_eq!(reports.day_of_month, DEFAULT_CLIENT_REPORT_DAY);
        assert_eq!(
            read::<RetentionSettings>(&json!({})).report_retention_months,
            DEFAULT_REPORT_RETENTION_MONTHS
        );
        // The group itself, as a request body, still needs every field.
        assert!(serde_json::from_value::<RetentionSettings>(json!({})).is_err());
        // A value of the wrong type does not read.
        let wrong = json!({ "report_retention_months": "twenty" }).to_string();
        assert!(read_group::<RetentionSettings>(&wrong).is_err());
    }

    #[test]
    fn sending_login_follows_the_same_login_switch() {
        let mut s = AppSettings::default();
        s.mailbox.username = "reports@example.com".into();
        s.mailbox.password = "mailbox secret".into();
        s.sending.group.username = "alerts@example.net".into();
        s.sending.password = "smtp secret".into();

        // Same login (the default): the mailbox's, sent from the mailbox address.
        let login = s.sending_login();
        assert_eq!(login.username, "reports@example.com");
        assert_eq!(login.password, "mailbox secret");
        assert_eq!(login.sender, "reports@example.com");
        assert!(!format!("{login:?}").contains("secret"));

        // A stored sender wins.
        s.sending.group.sender = "dmarc@example.com".into();
        assert_eq!(s.sending_login().sender, "dmarc@example.com");

        // A login that is no address gives no sender.
        s.sending.group.sender.clear();
        s.mailbox.username = "reports".into();
        assert_eq!(s.sending_login().sender, "");

        // Its own login: never the mailbox's, and no sender from it.
        s.sending.group.same_login_as_mailbox = false;
        let login = s.sending_login();
        assert_eq!(login.username, "alerts@example.net");
        assert_eq!(login.password, "smtp secret");
        assert_eq!(login.sender, "");
    }

    #[test]
    fn the_test_recipient_defaults_to_the_sender() {
        let mut s = AppSettings::default();
        assert_eq!(s.test_recipient(), "");
        s.mailbox.username = "reports@example.com".into();
        assert_eq!(s.test_recipient(), "reports@example.com");
        s.sending.group.sender = "dmarc@example.com".into();
        assert_eq!(s.test_recipient(), "dmarc@example.com");
        s.sending.group.test_recipient = "me@example.org".into();
        assert_eq!(s.test_recipient(), "me@example.org");
    }
}
