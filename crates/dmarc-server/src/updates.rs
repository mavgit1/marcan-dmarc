//! The version check: while `updates.check` is on, the server fetches
//! [`LATEST_URL`] (a static file on the docs site, `docs/public/latest.json`)
//! once a day and stores the answer; Settings → License says when it names
//! a newer version than the one running. On by default; while it is off
//! nothing is fetched and nothing is shown. The request carries nothing
//! about this installation: no version, no identifier, no cookie; the
//! comparison happens here. The browser never fetches the file.
//!
//! The job looks every [`UPDATE_CHECK_TICK`] (and when the switch is turned
//! on) and asks when the last attempt, answered or not, is a day old: it
//! survives restarts in the stored record, so a restarted server does not
//! ask again, and a site that is down is asked once a day, not every tick.

use dmarc_store::{
    get_update_check, load_settings, set_update_check, unix_now, SeenVersion, UpdateCheck,
    UpdateSettings,
};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::fmt;
use std::future::Future;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The one file the check fetches.
pub const LATEST_URL: &str = "https://marcan.kaitiaki.tech/latest.json";

/// The host the check talks to; a redirect elsewhere is refused.
const LATEST_HOST: &str = "marcan.kaitiaki.tech";

/// Where a release's notes are when the file names no usable URL.
pub const RELEASES_URL: &str = "https://marcan.kaitiaki.tech/releases";

/// How often the job looks whether a check is due.
pub const UPDATE_CHECK_TICK: Duration = Duration::from_hours(1);

/// The soonest a woken job (the switch just turned on) runs after its last run.
pub const UPDATE_CHECK_WAKE_GAP: Duration = Duration::from_secs(5);

/// One check a day: the next waits this long after the last attempt.
pub const UPDATE_CHECK_EVERY_SECS: i64 = 24 * 60 * 60;

/// The whole request, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `latest.json` is two fields; anything larger is not it.
const MAX_BYTES: usize = 4 * 1024;

/// The version that is running.
pub const RUNNING_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A release version, `X.Y.Z`, ordered as versions are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// `"0.2.1"`; `None` for anything else (a prefix, a suffix, a fourth part).
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut next = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self(next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// What `latest.json` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub version: Version,
    /// Its release notes: on the docs site, else [`RELEASES_URL`].
    pub url: String,
}

/// `{ "version": "X.Y.Z", "url": "..." }`. A URL that is not a page of the
/// docs site over HTTPS is replaced by [`RELEASES_URL`], so the link the UI
/// shows can only lead there.
pub fn parse_latest(body: &[u8]) -> Result<Latest, String> {
    #[derive(Deserialize)]
    struct Raw {
        version: String,
        #[serde(default)]
        url: String,
    }
    let raw: Raw = serde_json::from_slice(body).map_err(|e| format!("not latest.json: {e}"))?;
    let version = Version::parse(raw.version.trim())
        .ok_or_else(|| format!("not a version: {:?}", raw.version))?;
    let url = match reqwest::Url::parse(raw.url.trim()) {
        Ok(url) if on_docs_site(&url) => url.to_string(),
        _ => RELEASES_URL.to_owned(),
    };
    Ok(Latest { version, url })
}

/// `url` is on the docs site over HTTPS (the one host the check follows).
fn on_docs_site(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some(LATEST_HOST) && url.port().is_none()
}

/// The client: a short timeout, no cookies, no version in the user agent,
/// redirects only within the docs site (at most three).
fn client() -> Result<reqwest::Client, String> {
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 3 {
            attempt.error("too many redirects")
        } else if on_docs_site(attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("redirected away from the docs site")
        }
    });
    reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(5))
        .redirect(redirects)
        .user_agent("Marcan")
        .build()
        .map_err(|e| e.to_string())
}

/// Fetch and read `url` (the real check: [`LATEST_URL`]).
pub async fn fetch_latest(url: &str) -> Result<Latest, String> {
    let mut resp = client()?.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("answered {}", resp.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_BYTES {
            return Err(format!("larger than {MAX_BYTES} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    parse_latest(&body)
}

/// A check is due: none was attempted yet, the last is a day old, or the
/// clock went back past it.
pub fn due(last_attempt: Option<i64>, now: i64) -> bool {
    last_attempt.is_none_or(|at| now - at >= UPDATE_CHECK_EVERY_SECS || at > now)
}

/// The newer version to name, from the stored answer: only while the check
/// is on, and only when the answer is newer than `running`.
pub fn newer_version<'a>(
    settings: UpdateSettings,
    check: Option<&'a UpdateCheck>,
    running: &str,
) -> Option<&'a SeenVersion> {
    if !settings.check {
        return None;
    }
    let seen = check?.seen.as_ref()?;
    let newer = Version::parse(&seen.version)? > Version::parse(running)?;
    newer.then_some(seen)
}

/// The job, once: while the check is on and due, `fetch` and store the answer
/// (a failure keeps the last answer and waits a day like a success).
pub async fn run_once<F, Fut>(pool: &SqlitePool, now: i64, token: &CancellationToken, fetch: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Latest, String>>,
{
    let settings = match load_settings::<UpdateSettings>(pool).await {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!(error = %e, "version check: could not read its setting; skipping");
            return;
        }
    };
    if !settings.check {
        return;
    }
    let stored = match get_update_check(pool).await {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!(error = %e, "version check: could not read its record; skipping");
            return;
        }
    };
    if !due(stored.as_ref().map(|c| c.attempted_at), now) {
        return;
    }
    let answer = tokio::select! {
        () = token.cancelled() => return,
        answer = fetch() => answer,
    };
    let seen = match answer {
        Ok(latest) => {
            tracing::info!(latest = %latest.version, running = RUNNING_VERSION, "version check: answered");
            Some(SeenVersion {
                version: latest.version.to_string(),
                url: latest.url,
                checked_at: now,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, "version check failed; trying again in a day");
            stored.and_then(|c| c.seen)
        }
    };
    let record = UpdateCheck {
        attempted_at: now,
        seen,
    };
    if let Err(e) = set_update_check(pool, &record).await {
        tracing::warn!(error = %e, "version check: could not store the answer");
    }
}

/// The background job's run: [`run_once`] against [`LATEST_URL`].
pub async fn run(pool: SqlitePool, token: CancellationToken) {
    run_once(&pool, unix_now(), &token, || fetch_latest(LATEST_URL)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_store::{init, save_settings};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn versions_parse_strictly_and_order_numerically() {
        assert_eq!(Version::parse("0.2.1"), Some(Version(0, 2, 1)));
        for bad in [
            "",
            "0.2",
            "0.2.1.0",
            "v0.2.1",
            "0.2.1-rc1",
            "0..1",
            " 0.2.1",
            "a.b.c",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version::parse("0.10.0") > Version::parse("0.9.9"));
        assert!(Version::parse("1.0.0") > Version::parse("0.99.99"));
        assert_eq!(Version(0, 2, 1).to_string(), "0.2.1");
        assert!(Version::parse(RUNNING_VERSION).is_some());
    }

    #[test]
    fn latest_json_reads_and_its_link_stays_on_the_docs_site() {
        let latest = parse_latest(
            br#"{ "version": "0.3.0", "url": "https://marcan.kaitiaki.tech/releases#_0-3-0" }"#,
        )
        .unwrap();
        assert_eq!(latest.version, Version(0, 3, 0));
        assert_eq!(latest.url, "https://marcan.kaitiaki.tech/releases#_0-3-0");
        for url in [
            "javascript:alert(1)",
            "http://marcan.kaitiaki.tech/releases",
            "https://marcan.kaitiaki.tech.evil.example/releases",
            "https://marcan.kaitiaki.tech:8443/releases",
            "",
        ] {
            let body = serde_json::json!({ "version": "0.3.0", "url": url }).to_string();
            assert_eq!(
                parse_latest(body.as_bytes()).unwrap().url,
                RELEASES_URL,
                "{url}"
            );
        }
        assert_eq!(
            parse_latest(br#"{ "version": "0.3.0" }"#).unwrap().url,
            RELEASES_URL
        );
        assert!(parse_latest(br#"{ "version": "latest" }"#).is_err());
        assert!(parse_latest(b"<html>").is_err());
    }

    #[test]
    fn the_published_file_reads() {
        let file = include_bytes!("../../../docs/public/latest.json");
        let latest = parse_latest(file).unwrap();
        assert_eq!(latest.url, RELEASES_URL);
    }

    #[test]
    fn a_check_is_due_a_day_after_the_last_attempt() {
        let now = 1_800_000_000;
        assert!(due(None, now));
        assert!(!due(Some(now - 3600), now));
        assert!(!due(Some(now - UPDATE_CHECK_EVERY_SECS + 1), now));
        assert!(due(Some(now - UPDATE_CHECK_EVERY_SECS), now));
        assert!(due(Some(now + 60), now));
    }

    #[test]
    fn only_a_newer_version_is_named_and_only_while_on() {
        let check = |version: &str| UpdateCheck {
            attempted_at: 1,
            seen: Some(SeenVersion {
                version: version.into(),
                url: RELEASES_URL.into(),
                checked_at: 1,
            }),
        };
        let on = UpdateSettings { check: true };
        let newer = check("0.3.0");
        assert_eq!(
            newer_version(on, Some(&newer), "0.2.1").map(|s| s.version.as_str()),
            Some("0.3.0")
        );
        assert!(newer_version(UpdateSettings { check: false }, Some(&newer), "0.2.1").is_none());
        assert!(newer_version(on, Some(&check("0.2.1")), "0.2.1").is_none());
        assert!(newer_version(on, Some(&check("0.2.0")), "0.2.1").is_none());
        assert!(newer_version(on, None, "0.2.1").is_none());
    }

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&dir.path().join("t.db")).await.unwrap();
        (dir, pool)
    }

    /// The fetch for [`run_once`], counting its calls.
    fn answering(
        calls: &AtomicUsize,
        answer: Result<Latest, String>,
    ) -> impl FnOnce() -> std::future::Ready<Result<Latest, String>> + '_ {
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(answer)
        }
    }

    fn latest(version: &str) -> Result<Latest, String> {
        Ok(Latest {
            version: Version::parse(version).unwrap(),
            url: RELEASES_URL.into(),
        })
    }

    #[tokio::test]
    async fn nothing_is_fetched_while_the_check_is_off() {
        let (_dir, pool) = pool().await;
        save_settings(&pool, &UpdateSettings { check: false })
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);
        let token = CancellationToken::new();
        run_once(&pool, 1_000, &token, answering(&calls, latest("9.0.0"))).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(get_update_check(&pool).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_check_asks_once_a_day_and_keeps_its_last_answer() {
        let (_dir, pool) = pool().await;
        save_settings(&pool, &UpdateSettings { check: true })
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);
        let token = CancellationToken::new();
        let day = UPDATE_CHECK_EVERY_SECS;

        run_once(&pool, 1_000, &token, answering(&calls, latest("0.3.0"))).await;
        let stored = get_update_check(&pool).await.unwrap().unwrap();
        assert_eq!(stored.attempted_at, 1_000);
        let seen = stored.seen.unwrap();
        assert_eq!((seen.version.as_str(), seen.checked_at), ("0.3.0", 1_000));

        // Within the day (a restart, the next tick): not asked again.
        run_once(
            &pool,
            1_000 + day - 1,
            &token,
            answering(&calls, latest("0.4.0")),
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A day later, a failure: the last answer stays, the attempt counts.
        let failed = Err("unreachable".to_owned());
        run_once(&pool, 1_000 + day, &token, answering(&calls, failed)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let stored = get_update_check(&pool).await.unwrap().unwrap();
        assert_eq!(stored.attempted_at, 1_000 + day);
        assert_eq!(stored.seen.unwrap().version, "0.3.0");
        run_once(
            &pool,
            1_000 + day + 60,
            &token,
            answering(&calls, latest("0.4.0")),
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
