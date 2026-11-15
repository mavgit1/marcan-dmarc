//! The token source against the fake token endpoint: the signed assertion,
//! the cache and its refresh, coalescing, and every refusal in plain words.

use super::fake::{token, FakeTokenEndpoint, Registration};
use super::*;
use crate::accounts::certificate::tests::{other_certificate, public_key, test_certificate};
use crate::accounts::certificate::x5t_s256;
use crate::accounts::certificate::GeneratedCertificate;

pub(crate) const TENANT: &str = "11111111-2222-3333-4444-555555555555";
pub(crate) const CLIENT: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

/// The tenant's app registration with `cert` uploaded.
pub(crate) fn registration_with(cert: &GeneratedCertificate) -> Registration {
    Registration {
        tenant: TENANT.into(),
        client: CLIENT.into(),
        certificate: Some((public_key(&cert.private_key_pem), x5t_s256(&cert.cert_der))),
        ..Registration::default()
    }
}

/// The tenant's app registration with the test certificate uploaded.
pub(crate) fn registration() -> Registration {
    registration_with(test_certificate())
}

/// A token request proving itself with `cert`.
pub(crate) fn request_with(cert: &GeneratedCertificate) -> TokenRequest {
    TokenRequest {
        tenant_id: TENANT.into(),
        client_id: CLIENT.into(),
        certificate: ClientCertificate {
            private_key_pem: cert.private_key_pem.clone(),
            cert_der: cert.cert_der.clone(),
        },
    }
}

/// A token request proving itself with the test certificate.
pub(crate) fn certificate_request() -> TokenRequest {
    request_with(test_certificate())
}

#[tokio::test]
async fn a_certificate_gets_a_token_once_per_hour() {
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let source = endpoint.source();
    let request = certificate_request();
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(1)
    );
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(1)
    );
    assert_eq!(endpoint.grants(), 1);
    let form = &endpoint.requests()[0];
    assert_eq!(form["client_id"], CLIENT);
    assert_eq!(form["scope"], "https://graph.microsoft.com/.default");
    assert_eq!(form["grant_type"], "client_credentials");
    assert!(!form.contains_key("client_secret"));
    // The test asks anew whatever is cached.
    assert_eq!(source.fresh_token(&request).await.unwrap(), token(2));
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(2)
    );
}

#[tokio::test]
async fn a_certificate_gets_a_token_with_a_signed_assertion() {
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let source = endpoint.source();
    assert_eq!(
        source
            .token_lasting(&certificate_request(), REFRESH_BEFORE)
            .await
            .unwrap(),
        token(1)
    );
    let form = &endpoint.requests()[0];
    assert_eq!(
        form["client_assertion_type"],
        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"
    );
    // A certificate the registration does not have is refused.
    let endpoint = FakeTokenEndpoint::start(vec![Registration {
        certificate: None,
        ..registration()
    }])
    .await;
    let e = endpoint
        .source()
        .token_lasting(&certificate_request(), REFRESH_BEFORE)
        .await
        .unwrap_err();
    assert_eq!(e.kind, TokenErrorKind::AssertionRejected);
    assert!(e.detail.contains("AADSTS700027"), "{e}");
}

/// A token is used until five minutes before it expires (twenty for a
/// mailbox pass), then replaced.
#[tokio::test]
async fn tokens_are_refreshed_five_minutes_before_they_expire() {
    tokio::time::pause();
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    endpoint.set_expires_in(3600);
    let source = endpoint.source();
    let request = certificate_request();
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(1)
    );
    tokio::time::advance(Duration::from_secs(3600 - 300 - 1)).await;
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(1)
    );
    assert_eq!(endpoint.grants(), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(2)
    );
    assert_eq!(endpoint.grants(), 2);
    // A pass wants twenty minutes: a token with less is replaced for it,
    // and the fresh one serves the sends too.
    tokio::time::advance(Duration::from_mins(40)).await;
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(2)
    );
    assert_eq!(
        source
            .token_lasting(&request, PASS_REFRESH_BEFORE)
            .await
            .unwrap(),
        token(3)
    );
    assert_eq!(
        source
            .token_lasting(&request, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(3)
    );
}

/// The wall clock counts too: a token whose time ran out while the
/// monotonic clock stood still (the host asleep) is not used.
#[test]
fn a_token_expires_by_either_clock() {
    let now = Instant::now();
    let unix = 1_800_000_000;
    let cached = Cached {
        token: "t".into(),
        expires: now + Duration::from_secs(3600),
        expires_unix: unix + 3600,
    };
    assert!(cached.lasts(REFRESH_BEFORE, now, unix));
    // Slept for an hour: the monotonic clock did not move, the wall clock did.
    assert!(!cached.lasts(REFRESH_BEFORE, now, unix + 3600));
    assert!(!cached.lasts(REFRESH_BEFORE, now + Duration::from_secs(3600), unix));
    assert!(cached.lasts(PASS_REFRESH_BEFORE, now, unix + 3600 - 1201));
    assert!(!cached.lasts(PASS_REFRESH_BEFORE, now, unix + 3600 - 1200));
}

/// Two certificates of one app registration (two accounts) keep their own
/// tokens; a forgotten one is fetched anew.
#[tokio::test]
async fn two_certificates_of_one_registration_keep_their_tokens() {
    let mut both = registration();
    let other = registration_with(other_certificate());
    both.also = other.certificate;
    let endpoint = FakeTokenEndpoint::start(vec![both]).await;
    let source = endpoint.source();
    let (second, certificate) = (request_with(other_certificate()), certificate_request());
    for _ in 0..3 {
        source.token_lasting(&second, REFRESH_BEFORE).await.unwrap();
        source
            .token_lasting(&certificate, REFRESH_BEFORE)
            .await
            .unwrap();
    }
    assert_eq!(endpoint.grants(), 2, "each fetched once");
    assert_eq!(source.cache.lock().unwrap().len(), 2);

    source.forget(&second);
    assert_eq!(
        source.token_lasting(&second, REFRESH_BEFORE).await.unwrap(),
        token(3)
    );
    assert_eq!(
        source
            .token_lasting(&certificate, REFRESH_BEFORE)
            .await
            .unwrap(),
        token(2)
    );
    source.forget_login(&TENANT.to_uppercase(), CLIENT);
    assert!(source.cache.lock().unwrap().is_empty());
    source
        .token_lasting(&certificate, REFRESH_BEFORE)
        .await
        .unwrap();
    assert_eq!(endpoint.grants(), 4);
}

#[tokio::test]
async fn concurrent_requests_share_one_grant() {
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    endpoint.set_delay(Duration::from_millis(200));
    let source = Arc::new(endpoint.source());
    let request = certificate_request();
    let calls = (0..8).map(|_| {
        let (source, request) = (source.clone(), request.clone());
        tokio::spawn(async move { source.token_lasting(&request, REFRESH_BEFORE).await })
    });
    for call in futures::future::join_all(calls).await {
        assert_eq!(call.unwrap().unwrap(), token(1));
    }
    assert_eq!(endpoint.grants(), 1);
    assert_eq!(endpoint.requests().len(), 1);
}

async fn refusal(registration: Registration, request: &TokenRequest) -> TokenError {
    let endpoint = FakeTokenEndpoint::start(vec![registration]).await;
    endpoint
        .source()
        .token_lasting(request, REFRESH_BEFORE)
        .await
        .unwrap_err()
}

#[tokio::test]
async fn every_refusal_is_said_in_plain_words_with_its_code() {
    let check = |e: &TokenError, kind: TokenErrorKind, code: &str| {
        assert_eq!(e.kind, kind, "{e}");
        assert_eq!(e.message, token_error_sentence(kind));
        assert!(e.detail.contains(code), "{e}");
        assert!(!e.detail.contains("Trace ID"), "{e}");
    };
    let other_tenant = Registration {
        tenant: "99999999-2222-3333-4444-555555555555".into(),
        ..registration()
    };
    check(
        &refusal(other_tenant, &certificate_request()).await,
        TokenErrorKind::WrongTenant,
        "AADSTS90002",
    );
    let other_client = Registration {
        client: "99999999-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        ..registration()
    };
    check(
        &refusal(other_client, &certificate_request()).await,
        TokenErrorKind::UnknownClient,
        "AADSTS700016",
    );
    check(
        &refusal(registration(), &request_with(other_certificate())).await,
        TokenErrorKind::AssertionRejected,
        "AADSTS700027",
    );
    // A consent or resource refusal names no step of the Exchange role
    // setup: it is said as Microsoft's code and description.
    for (error, code) in [("invalid_grant", 65001), ("invalid_resource", 500_011)] {
        let refusing = Registration {
            refuse: Some((error, code, "The app has no permission.")),
            ..registration()
        };
        check(
            &refusal(refusing, &certificate_request()).await,
            TokenErrorKind::Refused,
            &format!("AADSTS{code}"),
        );
    }
    for code in [700_024, 700_023, 7_000_274] {
        let refusing = Registration {
            refuse: Some(("invalid_client", code, "The assertion is not valid.")),
            ..registration()
        };
        check(
            &refusal(refusing, &certificate_request()).await,
            TokenErrorKind::AssertionRejected,
            &format!("AADSTS{code}"),
        );
    }
    let disabled = Registration {
        refuse: Some(("unauthorized_client", 7_000_112, "Application is disabled.")),
        ..registration()
    };
    let e = refusal(disabled, &certificate_request()).await;
    check(&e, TokenErrorKind::Refused, "AADSTS7000112");
    assert_eq!(
        e.detail,
        "unauthorized_client: AADSTS7000112: Application is disabled."
    );
}

#[tokio::test]
async fn an_unreachable_endpoint_and_a_bad_tenant_id_ask_nothing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    drop(listener);
    let source = TokenSource::new(reqwest::Client::new(), closed);
    let e = source
        .token_lasting(&certificate_request(), REFRESH_BEFORE)
        .await
        .unwrap_err();
    assert_eq!(e.kind, TokenErrorKind::Unreachable);

    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let mut request = certificate_request();
    request.tenant_id = "contoso.com/../x".into();
    let e = endpoint
        .source()
        .token_lasting(&request, REFRESH_BEFORE)
        .await
        .unwrap_err();
    assert_eq!(e.kind, TokenErrorKind::WrongTenant);
    assert!(endpoint.requests().is_empty());
}

#[test]
fn guids_are_read_strictly() {
    assert!(is_guid(TENANT) && is_guid(&CLIENT.to_uppercase()));
    for bad in [
        "",
        "11111111222233334444555555555555",
        "11111111-2222-3333-4444-55555555555",
        "1111111g-2222-3333-4444-555555555555",
        " 11111111-2222-3333-4444-555555555555",
    ] {
        assert!(!is_guid(bad), "{bad:?}");
    }
    let printed = format!("{:?}", certificate_request());
    assert!(!printed.contains("PRIVATE KEY"), "{printed}");
}
