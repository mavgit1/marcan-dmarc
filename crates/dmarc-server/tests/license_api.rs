//! The operator license and branding: the key is checked before it is
//! stored and never answered, a valid key switches the notice off and
//! nothing else does, `GET /api/meta` says whether it is shown, and the
//! emails follow (the notice as the footer's last
//! sentence, the operator name as the From name).

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use dmarc_server::mail::MemorySender;
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{
    init, save_settings, AlertSettings, DigestFrequency, LicenseSettings, SendingGroup,
    SendingSettings, Weekday,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
mod common;
use common::{get_ok, post_bytes, post_json, put_json, send, TEST_OPERATOR_KEY};

/// The smallest PNG: one transparent pixel.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\0\x01\0\0\x05\0\x01\r\n-\xb4\0\0\0\0IEND\xaeB`\x82";

const NOTICE: &str = concat!("Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"));

struct App {
    _dir: tempfile::TempDir,
    pool: sqlx::SqlitePool,
    app: axum::Router,
    sender: Arc<MemorySender>,
}

async fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("test.db")).await.unwrap();
    let sender = Arc::new(MemorySender::default());
    let state = AppState::new(pool.clone()).with_mail_sender(sender.clone());
    // Sending through a made-up account (never contacted: the sender is stubbed).
    save_settings(
        &pool,
        &SendingSettings {
            group: SendingGroup {
                enabled: true,
                host: "smtp.example.com".into(),
                port: Some(465),
                same_login_as_mailbox: false,
                username: "alerts@example.com".into(),
                sender: "dmarc@example.com".into(),
                ..SendingGroup::default()
            },
            password: "secret".into(),
        },
    )
    .await
    .unwrap();
    save_settings(
        &pool,
        &AlertSettings {
            enabled: true,
            recipients: vec!["ops@example.com".into()],
            digest: DigestFrequency::Off,
            digest_hour: 7,
            digest_weekday: Weekday::Monday,
            base_url: String::new(),
            muted_kinds: Vec::new(),
        },
    )
    .await
    .unwrap();
    App {
        _dir: dir,
        pool,
        app: router_with_state(state),
        sender,
    }
}

async fn put_license(a: &App, body: Value) -> (StatusCode, Value) {
    put_json(&a.app, "/api/settings/license", body).await
}

#[tokio::test]
async fn without_a_key_the_notice_is_shown() {
    let a = app().await;
    let meta = get_ok(&a.app, "/api/meta").await;
    assert_eq!(meta["notice_shown"], true);
    assert_eq!(meta["licensed_to"], Value::Null);
    assert_eq!(
        meta["source_url"],
        concat!(
            "https://github.com/mavgit1/marcan-dmarc/tree/v",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(meta["license_url"], "https://marcan.kaitiaki.tech/license");
    assert_eq!(
        meta["billing_url"],
        "https://billing.stripe.com/p/login/3cIdRa2g90ar77I1vdeEo01"
    );
    // The UI carries no operator branding, and meta (read on every page) no logo.
    assert_eq!(meta.get("branding"), None);

    let settings = get_ok(&a.app, "/api/settings").await;
    assert_eq!(
        settings["license"],
        json!({ "has_key": false, "state": "no_key", "licensed_to": null, "key_hint": null })
    );
    assert_eq!(
        settings["branding"],
        json!({ "operator_name": "", "has_logo": false, "logo_type": null, "logo_bytes": null })
    );

    // Saving without a key keeps the notice; there is nothing to switch.
    let (status, body) = put_license(&a, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["license"]["state"], "no_key");
    assert_eq!(get_ok(&a.app, "/api/meta").await["notice_shown"], true);
}

#[tokio::test]
async fn an_invalid_key_is_refused_and_the_stored_key_stays() {
    let a = app().await;
    let (status, body) = put_license(&a, json!({ "operator_key": TEST_OPERATOR_KEY })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["license"]["licensed_to"], "test@example.com");
    assert_eq!(body["license"]["has_key"], true);
    // A valid key switches the notice off.
    assert_eq!(body["license"]["state"], "active");
    assert_eq!(get_ok(&a.app, "/api/meta").await["notice_shown"], false);
    // The key itself never leaves: only whose it is and the end of it.
    assert_eq!(body["license"]["key_hint"], "test@example.com · …IPPtCQ");
    let (_, signature) = TEST_OPERATOR_KEY.rsplit_once('.').unwrap();
    assert!(!body.to_string().contains(signature));
    let settings = get_ok(&a.app, "/api/settings").await;
    assert!(!settings.to_string().contains(signature));

    // A tampered key: 422, and the valid one is still stored.
    let tampered = TEST_OPERATOR_KEY.replace("test@", "admin@");
    let (status, body) = put_license(&a, json!({ "operator_key": tampered })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "This key is not valid.");
    let (status, body) = put_license(&a, json!({ "operator_key": "nonsense" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        get_ok(&a.app, "/api/settings").await["license"]["licensed_to"],
        "test@example.com"
    );
    assert_eq!(
        get_ok(&a.app, "/api/meta").await["licensed_to"],
        "test@example.com"
    );
}

#[tokio::test]
async fn a_key_switches_the_notice_off_and_removing_it_brings_it_back() {
    let a = app().await;
    let (status, body) = put_license(
        &a,
        json!({ "operator_key": format!("  {TEST_OPERATOR_KEY}\n") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["license"]["state"], "active");
    let meta = get_ok(&a.app, "/api/meta").await;
    assert_eq!(meta["notice_shown"], false);
    assert_eq!(meta["licensed_to"], "test@example.com");

    // Without `operator_key` the stored key is kept, and the notice stays off.
    let (status, body) = put_license(&a, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["license"]["licensed_to"], "test@example.com");
    assert_eq!(get_ok(&a.app, "/api/meta").await["notice_shown"], false);

    // Removing the key brings the notice back.
    let (status, body) = put_license(&a, json!({ "operator_key": "" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(get_ok(&a.app, "/api/meta").await["notice_shown"], true);
    assert_eq!(
        body["license"],
        json!({ "has_key": false, "state": "no_key", "licensed_to": null, "key_hint": null })
    );
}

#[tokio::test]
async fn a_stored_key_that_no_longer_verifies_is_invalid_and_the_notice_is_shown() {
    // Stored directly: the API refuses such a key, but one signed by a
    // rotated signing key reads the same.
    let a = app().await;
    save_settings(
        &a.pool,
        &LicenseSettings {
            operator_key: Some("test@example.com.AAAAbbbbCCCC".into()),
        },
    )
    .await
    .unwrap();
    let settings = get_ok(&a.app, "/api/settings").await;
    assert_eq!(
        settings["license"],
        json!({ "has_key": true, "state": "invalid", "licensed_to": null, "key_hint": "…bbCCCC" })
    );
    assert_eq!(get_ok(&a.app, "/api/meta").await["notice_shown"], true);
}

#[tokio::test]
async fn branding_is_free_and_validated() {
    let a = app().await;
    let (status, body) = put_json(
        &a.app,
        "/api/settings/branding",
        json!({ "operator_name": " Acme IT " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["branding"]["operator_name"], "Acme IT");
    let meta = get_ok(&a.app, "/api/meta").await;
    // The name is for the emails; the UI frame never shows it.
    assert_eq!(meta.get("branding"), None);
    // Branding never touches the notice.
    assert_eq!(meta["notice_shown"], true);

    let (status, body) = put_json(
        &a.app,
        "/api/settings/branding",
        json!({ "operator_name": "Acme\nIT" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "The operator name must be one line of text.");
}

#[tokio::test]
async fn the_branding_preview_follows_the_draft_and_the_license() {
    let a = app().await;
    // The draft's logo: its size as the browser read it, or none.
    let preview = |name: &str, logo: Value| {
        let app = a.app.clone();
        let body = json!({ "operator_name": name, "has_logo": !logo.is_null(), "logo_size": logo });
        async move { post_json(&app, "/api/settings/branding/preview", body).await }
    };
    let software = json!({
        "mark": { "width": 20, "height": 20 },
        "wordmark": "Marcan",
        "rest": ", free software"
    });
    let wide = json!({ "width": 1200, "height": 300 });
    // Nothing saved: Marcan's line alone, the notice signed by Marcan; with a
    // license there would be no letterhead at all.
    let (status, body) = preview(" ", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["operator_name"], Value::Null);
    assert_eq!(body["notice_shown"], true);
    let title = body["current"]["title"].as_str().unwrap().to_owned();
    assert!(
        title.starts_with("DMARC report for Example Ltd, "),
        "{title}"
    );
    assert_eq!(
        body["current"],
        json!({
            "notice_shown": true,
            "from_name": "Marcan",
            "top": { "software": software, "operator": null },
            "title": title,
            "sign_off": NOTICE
        })
    );
    assert_eq!(
        body["other"],
        json!({
            "notice_shown": false,
            "from_name": "Marcan",
            "top": { "software": null, "operator": null },
            "title": title,
            "sign_off": "Sent by Marcan."
        })
    );
    // A draft name, never saved: Marcan first, "Managed by" with the logo
    // fitted into 120 by 32 px; with a license the operator alone, the logo
    // fitted into 200 by 64 px.
    let (_, body) = preview(" Acme IT ", wide.clone()).await;
    assert_eq!(body["operator_name"], "Acme IT");
    assert_eq!(
        body["current"],
        json!({
            "notice_shown": true,
            "from_name": "Acme IT via Marcan",
            "top": {
                "software": software,
                "operator": {
                    "prominent": false,
                    "logo": { "width": 120, "height": 30 },
                    "name": "Managed by Acme IT"
                }
            },
            "title": title,
            "sign_off": concat!("Sent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"))
        })
    );
    assert_eq!(
        body["other"],
        json!({
            "notice_shown": false,
            "from_name": "Acme IT",
            "top": {
                "software": null,
                "operator": {
                    "prominent": true,
                    "logo": { "width": 200, "height": 50 },
                    "name": "Acme IT"
                }
            },
            "title": title,
            "sign_off": "Sent by Acme IT."
        })
    );
    assert_eq!(
        preview("Acme IT", Value::Null).await.1["current"]["top"]["operator"]["logo"],
        Value::Null
    );
    // The stored logo (1 by 1 pixels) at its own size: never enlarged.
    post_bytes(
        &a.app,
        "/api/settings/branding/logo",
        "image/png",
        PNG.to_vec(),
    )
    .await;
    let body = json!({ "operator_name": "Acme IT", "has_logo": true });
    let (_, body) = post_json(&a.app, "/api/settings/branding/preview", body).await;
    assert_eq!(
        body["current"]["top"]["operator"]["logo"],
        json!({ "width": 1, "height": 1 })
    );
    assert_eq!(
        get_ok(&a.app, "/api/settings").await["branding"]["operator_name"],
        ""
    );
    // A name that could not be saved is refused like the save.
    let (status, body) = preview("Acme\nIT", Value::Null).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // With a valid key the emails go out licensed; the other state is without.
    put_license(&a, json!({ "operator_key": TEST_OPERATOR_KEY })).await;
    let (_, body) = preview("Acme IT", Value::Null).await;
    assert_eq!(body["notice_shown"], false);
    assert_eq!(body["current"]["sign_off"], "Sent by Acme IT.");
    assert_eq!(body["current"]["from_name"], "Acme IT");
    assert_eq!(body["current"]["top"]["software"], Value::Null);
    assert_eq!(body["other"]["from_name"], "Acme IT via Marcan");
    assert_eq!(body["other"]["top"]["software"], software);
    assert_eq!(
        preview("", Value::Null).await.1["current"]["sign_off"],
        "Sent by Marcan."
    );
}

#[tokio::test]
async fn the_logo_is_uploaded_checked_by_its_bytes_and_removed() {
    let a = app().await;
    assert_eq!(logo(&a, None).await.0, StatusCode::NOT_FOUND);
    // The declared type does not matter: the bytes are a PNG.
    let (status, body) = post_bytes(
        &a.app,
        "/api/settings/branding/logo",
        "image/jpeg",
        PNG.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["branding"],
        json!({ "operator_name": "", "has_logo": true, "logo_type": "png", "logo_bytes": PNG.len() })
    );
    // The image is its own GET, with its type and a tag that revalidates it.
    let (status, headers, bytes) = logo(&a, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    let (status, headers, bytes) = logo(&a, Some(&etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(headers[header::ETAG], etag.as_str());
    assert!(bytes.is_empty());

    // Saving the name keeps the logo.
    put_json(
        &a.app,
        "/api/settings/branding",
        json!({ "operator_name": "Acme IT" }),
    )
    .await;
    assert_eq!(logo(&a, Some(&etag)).await.0, StatusCode::NOT_MODIFIED);

    // Not an image, too big, or a form: refused, the logo stays.
    let (status, body) = post_bytes(
        &a.app,
        "/api/settings/branding/logo",
        "image/png",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "The logo must be a PNG, JPEG or WebP image.");
    let mut big = PNG.to_vec();
    big.resize(300 * 1024, 0);
    let (status, body) = post_bytes(&a.app, "/api/settings/branding/logo", "image/png", big).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("The logo is 300 KB; it can be at most 200 KB."),
        "{body}"
    );
    let (status, _) = post_bytes(
        &a.app,
        "/api/settings/branding/logo",
        "multipart/form-data; boundary=x",
        PNG.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(logo(&a, Some(&etag)).await.0, StatusCode::NOT_MODIFIED);

    let (status, body) = send(&a.app, "DELETE", "/api/settings/branding/logo", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["branding"]["has_logo"], false);
    assert_eq!(body["branding"]["operator_name"], "Acme IT");
    assert_eq!(logo(&a, Some(&etag)).await.0, StatusCode::NOT_FOUND);
}

/// `(status, headers, body)` of `GET /api/settings/branding/logo`, sent with
/// `If-None-Match` when `etag` is given.
async fn logo(a: &App, etag: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::builder().uri("/api/settings/branding/logo");
    if let Some(etag) = etag {
        request = request.header(header::IF_NONE_MATCH, etag);
    }
    let response = a
        .app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (parts.status, parts.headers, bytes)
}

#[tokio::test]
async fn the_emails_carry_the_notice_and_the_operator_name() {
    let a = app().await;
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = a.sender.sent();
    assert!(
        sent[0].text.ends_with(&format!("\n\n{NOTICE}\n")),
        "{}",
        sent[0].text
    );
    assert!(sent[0].html.as_deref().unwrap().contains(
        concat!("Sent by Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: <a href=\"https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "\"")
    ));
    // Without a name, the email is from Marcan.
    assert_eq!(a.sender.from_names(), ["Marcan"]);

    // A name signs the notice.
    put_json(
        &a.app,
        "/api/settings/branding",
        json!({ "operator_name": "Acme IT" }),
    )
    .await;
    post_bytes(
        &a.app,
        "/api/settings/branding/logo",
        "image/png",
        PNG.to_vec(),
    )
    .await;
    post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    let sent = a.sender.sent();
    // Without a key Marcan opens the email and the operator follows.
    assert!(
        sent[1].text.starts_with(
            "Marcan, free software\nManaged by Acme IT\n\n4 changes on example.com\n\n"
        ),
        "{}",
        sent[1].text
    );
    let cids: Vec<&str> = sent[1].images.iter().map(|i| i.cid).collect();
    assert_eq!(cids, ["marcan-mark", "logo"]);
    assert!(
        sent[1].text.ends_with(concat!("\n\nSent by Acme IT with Marcan, free software you may run yourself under the Beneficiary Copyleft License 1.0, source code: https://github.com/mavgit1/marcan-dmarc/tree/v", env!("CARGO_PKG_VERSION"), "\n")),
        "{}",
        sent[1].text
    );

    // A valid key switches the notice off: the operator signs alone.
    let (status, _) = put_license(&a, json!({ "operator_key": TEST_OPERATOR_KEY })).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = a.sender.sent();
    let branded = &sent[2];
    assert!(
        branded
            .text
            .starts_with("Acme IT\n\n4 changes on example.com\n\n"),
        "{}",
        branded.text
    );
    assert!(
        branded.text.ends_with("\n\nSent by Acme IT.\n"),
        "{}",
        branded.text
    );
    assert!(!branded.text.contains("free software"));
    let html = branded.html.as_deref().unwrap();
    // The logo goes along as an inline attachment the HTML names by Content-ID.
    // At its own size, as attributes for desktop Outlook and as CSS, in one
    // row with the name beside it (which names it, so its alt is empty).
    assert!(html.contains(
        "<img src=\"cid:logo\" alt=\"\" width=\"1\" height=\"1\" style=\"width:1px;height:1px;display:block;border:0\"></td>\
         <td style=\"vertical-align:middle;padding:0 0 0 12px;font-size:20px;font-weight:700;color:#111827\">Acme IT</td></tr></table>"
    ), "{html}");
    let cids: Vec<&str> = branded.images.iter().map(|i| i.cid).collect();
    assert_eq!(cids, ["logo"], "the logo alone, no Marcan mark");
    let logo = &branded.images[0];
    assert_eq!((logo.media_type, logo.bytes.as_slice()), ("image/png", PNG));
    assert!(!html.contains("free software"));
    // The From name: the operator via Marcan without a key, the operator alone with one.
    assert_eq!(
        a.sender.from_names(),
        ["Marcan", "Acme IT via Marcan", "Acme IT"]
    );
}
