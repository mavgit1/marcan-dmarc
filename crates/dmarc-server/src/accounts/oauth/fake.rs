//! A fake Microsoft token endpoint on 127.0.0.1 for the tests: tenants with
//! app registrations, each with an uploaded certificate's public key,
//! answering the client credentials grant as Entra does (its error
//! shape and AADSTS codes), and counting the grants it answered.

use crate::accounts::certificate::tests::decode;
use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::Url;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One app registration in a fake tenant.
#[derive(Clone, Default)]
pub(crate) struct Registration {
    pub tenant: String,
    pub client: String,
    /// The uploaded certificate: its public key (PKCS#1 DER) and its
    /// `x5t#S256`.
    pub certificate: Option<(Vec<u8>, String)>,
    /// A second uploaded certificate (Entra keeps several).
    pub also: Option<(Vec<u8>, String)>,
    /// Refuse every grant with this error, code and description.
    pub refuse: Option<(&'static str, i64, &'static str)>,
}

#[derive(Default)]
struct FakeState {
    registrations: Mutex<Vec<Registration>>,
    grants: AtomicUsize,
    expires_in: AtomicU64,
    delay_ms: AtomicU64,
    base: Mutex<String>,
    /// The form fields of every request, in order.
    requests: Mutex<Vec<HashMap<String, String>>>,
}

/// The running fake endpoint.
#[derive(Clone)]
pub(crate) struct FakeTokenEndpoint {
    pub base: Url,
    state: Arc<FakeState>,
}

fn refusal(status: StatusCode, error: &str, code: i64, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error,
            "error_description": format!("AADSTS{code}: {description}\r\nTrace ID: 0\r\nCorrelation ID: 0\r\nTimestamp: 2026-10-28 00:00:00Z"),
            "error_codes": [code],
        })),
    )
        .into_response()
}

/// The token `n` of a registration: a JWT (unsigned) naming no
/// permission, as one the app's Exchange role assignments give access with.
pub(crate) fn token(n: usize) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
    let claims = URL_SAFE_NO_PAD.encode(serde_json::json!({"n": n}).to_string());
    format!("{header}.{claims}.sig")
}

async fn grant(
    State(state): State<Arc<FakeState>>,
    Path(tenant): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    state.requests.lock().unwrap().push(form.clone());
    let delay = state.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let field = |k: &str| form.get(k).cloned().unwrap_or_default();
    let registrations = state.registrations.lock().unwrap().clone();
    if !registrations.iter().any(|r| r.tenant == tenant) {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            90002,
            &format!("Tenant '{tenant}' not found."),
        );
    }
    let client = field("client_id");
    let Some(reg) = registrations
        .iter()
        .find(|r| r.tenant == tenant && r.client == client)
    else {
        return refusal(
            StatusCode::BAD_REQUEST,
            "unauthorized_client",
            700_016,
            &format!("Application with identifier '{client}' was not found in the directory."),
        );
    };
    if field("scope") != super::GRAPH_SCOPE || field("grant_type") != "client_credentials" {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            1_002_012,
            "Bad scope.",
        );
    }
    if let Some((error, code, description)) = reg.refuse {
        return refusal(StatusCode::BAD_REQUEST, error, code, description);
    }
    let assertion = field("client_assertion");
    let ok = field("client_assertion_type")
        == "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"
        && [&reg.certificate, &reg.also]
            .into_iter()
            .flatten()
            .any(|(public, x5t)| {
                let (header, claims, verified) = decode(&assertion, public);
                let aud = format!("{}/{tenant}/oauth2/v2.0/token", state.base.lock().unwrap());
                verified
                    && header["alg"] == "RS256"
                    && header["x5t#S256"] == x5t.as_str()
                    && claims["aud"] == aud.as_str()
                    && claims["iss"] == client.as_str()
                    && claims["sub"] == client.as_str()
                    && claims["exp"].as_i64() > Some(dmarc_store::unix_now())
            });
    if !ok {
        return refusal(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            700_027,
            "Client assertion failed signature validation.",
        );
    }
    let n = state.grants.fetch_add(1, Ordering::SeqCst) + 1;
    Json(serde_json::json!({
        "token_type": "Bearer",
        "expires_in": state.expires_in.load(Ordering::SeqCst),
        "ext_expires_in": state.expires_in.load(Ordering::SeqCst),
        "access_token": token(n),
    }))
    .into_response()
}

impl FakeTokenEndpoint {
    /// An endpoint with `registrations`, its tokens valid an hour.
    pub(crate) async fn start(registrations: Vec<Registration>) -> Self {
        let state = Arc::new(FakeState {
            registrations: Mutex::new(registrations),
            expires_in: AtomicU64::new(3599),
            ..FakeState::default()
        });
        let app = Router::new()
            .route("/{tenant}/oauth2/v2.0/token", post(grant))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        state.base.lock().unwrap().clone_from(&base);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self {
            base: Url::parse(&base).unwrap(),
            state,
        }
    }

    /// A token source for this endpoint (no timeouts and no idle
    /// connections, so a paused test clock only moves when told to).
    pub(crate) fn source(&self) -> super::TokenSource {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        super::TokenSource::new(http, self.base.clone())
    }

    /// Tokens issued so far.
    pub(crate) fn grants(&self) -> usize {
        self.state.grants.load(Ordering::SeqCst)
    }

    /// Requests answered so far, tokens or refusals.
    pub(crate) fn requests(&self) -> Vec<HashMap<String, String>> {
        self.state.requests.lock().unwrap().clone()
    }

    pub(crate) fn set_expires_in(&self, secs: u64) {
        self.state.expires_in.store(secs, Ordering::SeqCst);
    }

    /// Wait this long before answering (in tokio time).
    pub(crate) fn set_delay(&self, delay: Duration) {
        let ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.state.delay_ms.store(ms, Ordering::SeqCst);
    }
}
