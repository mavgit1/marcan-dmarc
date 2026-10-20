//! The client reports settings group and the recipient list: off by
//! default, validated, and a list that replaces the stored recipients whole,
//! with a preview that says who starts and who stops.

use axum::http::StatusCode;
use serde_json::{json, Value};
mod common;
use common::{get_ok, post_json, put_json, seed_organizations, TestApp};

async fn app() -> TestApp {
    let t = TestApp::new().await;
    // Acme (example.com) and Beta (other.org), plus Default.
    seed_organizations(&t.pool).await;
    t
}

async fn preview(t: &TestApp, text: &str) -> Value {
    let (status, body) = post_json(
        &t.app,
        "/api/settings/client-reports/recipients/preview",
        json!({ "text": text }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn apply(t: &TestApp, text: &str) -> (StatusCode, Value) {
    put_json(
        &t.app,
        "/api/settings/client-reports/recipients",
        json!({ "text": text }),
    )
    .await
}

fn emails(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                r["organization"].as_str().unwrap(),
                r["email"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn settings_are_off_by_default_and_validated() {
    let t = app().await;
    let group = get_ok(&t.app, "/api/settings").await["client_reports"].clone();
    assert_eq!(
        group,
        json!({
            "enabled": false,
            "day_of_month": 1,
            "hour_utc": 7,
            "reply_to": "",
            "next_send_at": null,
            "organizations_due": 0,
            "last_sent_at": null,
            "held_reason": null,
            "undelivered": null,
        })
    );

    let body = json!({ "enabled": true, "day_of_month": 5, "hour_utc": 18, "reply_to": " help@msp.example " });
    // On with nobody to send to would send nothing: refused until the list has someone.
    let (status, err) = put_json(&t.app, "/api/settings/client-reports", body.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(
        err["error"],
        "Add at least one recipient to the list first."
    );
    let off = json!({ "enabled": false, "day_of_month": 5, "hour_utc": 18, "reply_to": "" });
    let (status, _) = put_json(&t.app, "/api/settings/client-reports", off).await;
    assert_eq!(status, StatusCode::OK, "off with nobody listed is fine");

    let (status, _) = apply(&t, "Acme,it@acme.example").await;
    assert_eq!(status, StatusCode::OK);
    let (status, saved) = put_json(&t.app, "/api/settings/client-reports", body).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let mut group = saved["client_reports"].clone();
    // On: the next send is the 5th at 18:00 UTC, for the one organization listed.
    let next = group["next_send_at"].take().as_i64().expect("a next send");
    assert_eq!(next % 86_400, 18 * 3600);
    assert_eq!(
        group,
        json!({
            "enabled": true,
            "day_of_month": 5,
            "hour_utc": 18,
            "reply_to": "help@msp.example",
            "next_send_at": null,
            "organizations_due": 1,
            "last_sent_at": null,
            "held_reason": null,
            "undelivered": null,
        })
    );

    for bad in [
        json!({ "enabled": true, "day_of_month": 31, "hour_utc": 7, "reply_to": "" }),
        json!({ "enabled": true, "day_of_month": 1, "hour_utc": 24, "reply_to": "" }),
        json!({ "enabled": true, "day_of_month": 1, "hour_utc": 7, "reply_to": "help" }),
    ] {
        let (status, err) = put_json(&t.app, "/api/settings/client-reports", bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    }
    assert_eq!(
        get_ok(&t.app, "/api/settings").await["client_reports"]["day_of_month"],
        5,
        "a rejected save changes nothing"
    );
}

#[tokio::test]
async fn the_preview_names_line_errors_with_a_suggestion() {
    let t = app().await;
    let body = preview(
        &t,
        "organization,email\n# Acme's IT\nacme,it@example.com\nAcmee,boss@example.com\nBeta,not-an-address\n",
    )
    .await;
    assert_eq!(
        body["errors"],
        json!([
            { "line": 4, "message": "no organization is named Acmee: did you mean Acme?" },
            { "line": 5, "message": "not a valid email address: not-an-address" },
        ])
    );
    assert_eq!(emails(&body["changes"]["start"]), ["Acme it@example.com"]);

    // A list with errors is not saved, and no organization is created.
    let (status, err) = apply(&t, "Acmee,boss@example.com\n").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["errors"][0]["line"], 1);
    let scope = get_ok(&t.app, "/api/scope").await;
    let names: Vec<&str> = scope["organizations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Default", "Acme", "Beta"]);
    assert_eq!(
        get_ok(&t.app, "/api/settings/client-reports/recipients").await["text"],
        ""
    );
}

#[tokio::test]
async fn the_list_is_the_full_truth() {
    let t = app().await;
    let (status, saved) = apply(
        &t,
        "Beta,ops@other.org\nAcme,it@example.com\nAcme,boss@example.com\n",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["changes"]["start"].as_array().unwrap().len(), 3);

    let list = get_ok(&t.app, "/api/settings/client-reports/recipients").await;
    assert_eq!(
        list["text"],
        "Acme,boss@example.com\nAcme,it@example.com\nBeta,ops@other.org\n"
    );

    // Saving the same list is no change.
    let same = preview(&t, list["text"].as_str().unwrap()).await;
    assert_eq!(same["changes"]["start"], json!([]));
    assert_eq!(same["changes"]["stop"], json!([]));
    assert_eq!(same["changes"]["unchanged_count"], 3);

    // A line taken out stops that report; a new one starts.
    let next = "Acme,it@example.com\nBeta,ops@other.org\nBeta,cto@other.org\n";
    let changes = preview(&t, next).await["changes"].clone();
    assert_eq!(emails(&changes["start"]), ["Beta cto@other.org"]);
    assert_eq!(emails(&changes["stop"]), ["Acme boss@example.com"]);
    assert_eq!(changes["unchanged_count"], 2);
    let (status, applied) = apply(&t, next).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(applied["changes"], changes);

    // An empty list stops every report (the reports are off).
    let (_, cleared) = apply(&t, "").await;
    assert_eq!(cleared["changes"]["stop"].as_array().unwrap().len(), 3);
    assert_eq!(
        get_ok(&t.app, "/api/settings/client-reports/recipients").await["text"],
        ""
    );
}

#[tokio::test]
async fn the_list_is_not_emptied_while_the_reports_are_on() {
    let t = app().await;
    let (status, _) = apply(&t, "Acme,it@example.com\n").await;
    assert_eq!(status, StatusCode::OK);
    let on = json!({ "enabled": true, "day_of_month": 1, "hour_utc": 7, "reply_to": "" });
    let (status, err) = put_json(&t.app, "/api/settings/client-reports", on).await;
    assert_eq!(status, StatusCode::OK, "{err}");

    // On, an empty list would send nothing: refused like switching on with
    // one. A list of comments alone is empty too.
    for empty in ["", "# nobody for now\n"] {
        let (status, err) = apply(&t, empty).await;
        assert_eq!(status, StatusCode::CONFLICT, "{err}");
        assert_eq!(
            err["error"],
            "Add at least one recipient to the list first."
        );
    }
    assert_eq!(
        get_ok(&t.app, "/api/settings/client-reports/recipients").await["text"],
        "Acme,it@example.com\n",
        "a refused save changes nothing"
    );

    // Off first, then the list may be emptied.
    let off = json!({ "enabled": false, "day_of_month": 1, "hour_utc": 7, "reply_to": "" });
    let (status, _) = put_json(&t.app, "/api/settings/client-reports", off).await;
    assert_eq!(status, StatusCode::OK);
    let (status, cleared) = apply(&t, "").await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["changes"]["stop"].as_array().unwrap().len(), 1);
}
