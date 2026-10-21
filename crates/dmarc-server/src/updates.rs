//! The version check: while `updates.check` is on, the server fetches
//! [`LATEST_URL`] (`docs/public/latest.json` on the docs site) once a day
//! and stores the answer; Settings → License names a newer version. The
//! request carries nothing about this installation, and the browser never
//! fetches the file. The job wakes every [`UPDATE_CHECK_TICK`] (and when the
//! switch is turned on) and asks once the last attempt, answered or not, is
//! a day old, so neither a restart nor a site that is down asks more often.

use dmarc_store::{
    get_update_check, load_settings, set_update_check, unix_now, SeenVersion, StoreError,
    UpdateCheck, UpdateSettings,
};
use sqlx::SqlitePool;
use std::future::Future;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const LATEST_URL: &str = "https://marcan.kaitiaki.tech/latest.json";
/// The link when the file names none on the docs site.
const RELEASES_URL: &str = "https://marcan.kaitiaki.tech/releases";
const DAY_SECS: i64 = 24 * 60 * 60;
const MAX_BYTES: usize = 4 * 1024;
/// The version and the link to its release notes, or why there is none.
type Answer = Result<(String, String), String>;

pub const UPDATE_CHECK_TICK: Duration = Duration::from_hours(1);
/// The soonest a woken job (the switch just turned on) runs after its last run.
pub const UPDATE_CHECK_WAKE_GAP: Duration = Duration::from_secs(5);
pub const RUNNING_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `X.Y.Z` as a tuple that orders as versions do; `None` for anything else.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    // Digits only: `u64::from_str` also takes a leading `+`.
    let num = |p: &str| p.parse().ok().filter(|_| !p.starts_with('+'));
    let mut parts = text.split('.').map(num);
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// `{ "version": "X.Y.Z", "url": "..." }` as (version, link). A link that is
/// not an HTTPS page of the docs site becomes [`RELEASES_URL`].
fn parse_latest(body: &[u8]) -> Answer {
    #[derive(serde::Deserialize)]
    struct Raw {
        version: String,
        url: Option<String>,
    }
    let raw: Raw = serde_json::from_slice(body).map_err(|e| format!("not latest.json: {e}"))?;
    let (x, y, z) =
        version(raw.version.trim()).ok_or_else(|| format!("not a version: {:?}", raw.version))?;
    let url = match reqwest::Url::parse(raw.url.unwrap_or_default().trim()) {
        Ok(url) if on_docs_site(&url) => url.to_string(),
        _ => RELEASES_URL.to_owned(),
    };
    Ok((format!("{x}.{y}.{z}"), url))
}

/// The one host the check talks to, redirects included.
fn on_docs_site(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("marcan.kaitiaki.tech")
        && url.port().is_none()
}

async fn fetch_latest() -> Answer {
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() < 3 && on_docs_site(attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("too many redirects, or away from the docs site")
        }
    });
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(redirects)
        .user_agent("Marcan")
        .build()
        .map_err(|e| e.to_string())?;
    let send = client.get(LATEST_URL).send();
    let mut resp = send.await.map_err(|e| e.to_string())?;
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

/// The stored answer, while the check is on and it is newer than `running`.
pub fn newer_version<'a>(
    settings: UpdateSettings,
    check: Option<&'a UpdateCheck>,
    running: &str,
) -> Option<&'a SeenVersion> {
    let seen = check?.seen.as_ref()?;
    (settings.check && version(&seen.version)? > version(running)?).then_some(seen)
}

/// While on and due, await `fetch` and store the answer; a failure keeps
/// the last answer and waits a day like a success.
async fn run_once(
    pool: &SqlitePool,
    now: i64,
    token: &CancellationToken,
    fetch: impl Future<Output = Answer>,
) -> Result<(), StoreError> {
    if !load_settings::<UpdateSettings>(pool).await?.check {
        return Ok(());
    }
    let stored = get_update_check(pool).await?;
    // Due: none attempted yet, the last a day old, or the clock went back past it.
    let last = stored.as_ref().map(|c| c.attempted_at);
    if last.is_some_and(|at| at <= now && now - at < DAY_SECS) {
        return Ok(());
    }
    let seen = match tokio::select! {
        () = token.cancelled() => return Ok(()),
        answer = fetch => answer,
    } {
        Ok((version, url)) => {
            tracing::info!(latest = %version, running = RUNNING_VERSION, "version check: answered");
            let checked_at = now;
            Some(SeenVersion {
                version,
                url,
                checked_at,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, "version check failed; trying again in a day");
            stored.and_then(|c| c.seen)
        }
    };
    let attempted_at = now;
    set_update_check(pool, &UpdateCheck { attempted_at, seen }).await
}

pub async fn run(pool: SqlitePool, token: CancellationToken) {
    if let Err(e) = run_once(&pool, unix_now(), &token, fetch_latest()).await {
        tracing::warn!(error = %e, "version check: could not read or store its record");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_store::{init, save_settings};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    #[test]
    fn versions_and_latest_json_parse_strictly() {
        assert!(version("0.10.0") > version("0.9.9") && version("1.0.0") > version("0.99.99"));
        assert!(version(RUNNING_VERSION).is_some());
        for bad in "|0.2|0.2.1.0|v0.2.1|0.2.1-rc1|0..1| 0.2.1|a.b.c|+0.2.1".split('|') {
            assert_eq!(version(bad), None, "{bad:?}");
        }
        let read = |url: &str| {
            let body = json!({ "version": "0.3.0", "url": url }).to_string();
            parse_latest(body.as_bytes()).unwrap()
        };
        let notes = "https://marcan.kaitiaki.tech/releases#_0-3-0";
        assert_eq!(read(notes), ("0.3.0".into(), notes.into()));
        for url in [
            "javascript:alert(1)",
            "http://marcan.kaitiaki.tech/releases",
            "https://marcan.kaitiaki.tech.evil.example/releases",
            "https://marcan.kaitiaki.tech:8443/releases",
            "",
        ] {
            assert_eq!(read(url).1, RELEASES_URL, "{url}");
        }
        let published = include_bytes!("../../../docs/public/latest.json");
        for body in [&br#"{ "version": "0.3.0" }"#[..], published] {
            assert_eq!(parse_latest(body).unwrap().1, RELEASES_URL);
        }
        assert!(parse_latest(br#"{ "version": "latest" }"#).is_err());
        assert!(parse_latest(b"<html>").is_err());
    }

    #[test]
    fn only_a_newer_version_is_named_and_only_while_on() {
        let named = |check, version: &str| {
            let seen = json!({ "version": version, "url": "", "checked_at": 1 });
            let record = serde_json::from_value(json!({ "attempted_at": 1, "seen": seen }));
            newer_version(UpdateSettings { check }, Some(&record.unwrap()), "0.2.1").is_some()
        };
        assert!(named(true, "0.3.0"));
        assert!(!named(false, "0.3.0") && !named(true, "0.2.1") && !named(true, "0.2.0"));
        assert!(newer_version(UpdateSettings { check: true }, None, "0.2.1").is_none());
    }

    async fn answer(calls: &AtomicUsize, version: Option<&str>) -> Answer {
        calls.fetch_add(1, SeqCst);
        Ok((version.ok_or("unreachable")?.into(), RELEASES_URL.into()))
    }

    #[tokio::test]
    async fn the_check_asks_once_a_day_only_while_on_and_keeps_its_last_answer() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&dir.path().join("t.db")).await.unwrap();
        let (calls, token) = (AtomicUsize::new(0), CancellationToken::new());
        let run = async |now, v| run_once(&pool, now, &token, answer(&calls, v)).await;
        let switch = async |check| save_settings(&pool, &UpdateSettings { check }).await;
        let (day, stored) = (DAY_SECS, || get_update_check(&pool));

        switch(false).await.unwrap();
        run(1_000, Some("9.0.0")).await.unwrap();
        assert_eq!((calls.load(SeqCst), stored().await.unwrap()), (0, None));

        switch(true).await.unwrap();
        run(1_000, Some("0.3.0")).await.unwrap();
        let seen = stored().await.unwrap().unwrap().seen.unwrap();
        assert_eq!((seen.version.as_str(), seen.checked_at), ("0.3.0", 1_000));
        // Within the day (a restart, the next tick): not asked again.
        run(1_000 + day - 1, Some("0.4.0")).await.unwrap();
        assert_eq!(calls.load(SeqCst), 1);
        // A day later, a failure: the last answer stays, the attempt counts.
        run(1_000 + day, None).await.unwrap();
        let r = stored().await.unwrap().unwrap();
        assert_eq!((r.attempted_at, r.seen), (1_000 + day, Some(seen)));
        run(1_000 + day + 60, Some("0.4.0")).await.unwrap();
        assert_eq!(calls.load(SeqCst), 2);
        // The clock went back past the last attempt: asked again.
        run(1_000, Some("0.4.0")).await.unwrap();
        assert_eq!(calls.load(SeqCst), 3);
    }
}
