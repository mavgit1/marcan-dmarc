//! The MTA-STS policy fetch: the HTTP client (through the checks' own
//! resolver, which refuses private addresses; no proxy, no redirects),
//! `GET https://mta-sts.<domain>/.well-known/mta-sts.txt` with a size cap,
//! and which [`PolicyFetch`] a failed fetch is.

use super::DnsChecker;
use crate::dns::ResolveFailure;
use crate::policy::PolicyFetch;
use crate::DnsError;
use std::sync::Arc;
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

impl DnsChecker {
    /// `GET https://mta-sts.<domain>/.well-known/mta-sts.txt`: the body when
    /// read, and how the fetch went.
    pub(super) async fn fetch_policy(&self, domain: &str) -> (Option<String>, PolicyFetch) {
        let port = match self.policy_port {
            443 => String::new(),
            p => format!(":{p}"),
        };
        let url = format!("https://mta-sts.{domain}{port}/.well-known/mta-sts.txt");
        match self.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                match read_capped(resp).await {
                    Some(t) => (Some(t), PolicyFetch::Ok { content_type }),
                    None => (None, PolicyFetch::TooLarge),
                }
            }
            Ok(resp) if resp.status().is_redirection() => {
                let location = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                (None, PolicyFetch::Redirect { location })
            }
            Ok(resp) => (
                None,
                PolicyFetch::HttpStatus {
                    code: resp.status().as_u16(),
                },
            ),
            Err(e) => (None, classify_fetch_error(&e)),
        }
    }
}

/// The client for the MTA-STS policy fetch: resolving through `resolver`
/// (the checks' nameservers, refusing private addresses), trusting
/// `extra_root` on top of the system roots (tests only), never following
/// redirects, never through a proxy.
pub(super) fn http_client<R: reqwest::dns::Resolve + 'static>(
    resolver: Arc<R>,
    extra_root: Option<reqwest::Certificate>,
) -> Result<reqwest::Client, DnsError> {
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .dns_resolver(resolver)
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(Duration::from_secs(5))
        // RFC 8461 §3.3: the policy fetch must not follow redirects. Following
        // them would also let any domain send this request elsewhere.
        .redirect(reqwest::redirect::Policy::none())
        // A proxy (HTTPS_PROXY in the environment) would resolve the name
        // itself, past the private-address check, so the client connects
        // directly to the addresses the resolver handed it.
        .no_proxy()
        .user_agent(concat!("Marcan/", env!("CARGO_PKG_VERSION")));
    if let Some(root) = extra_root {
        builder = builder.add_root_certificate(root);
    }
    builder.build().map_err(|e| DnsError::Init(e.to_string()))
}

/// Which [`PolicyFetch`] a transport error is. reqwest folds DNS and TLS
/// failures into "connect" errors, so the cause is found by type along the
/// error chain: our own resolver's [`ResolveFailure`], or the `rustls::Error`
/// that tokio-rustls wraps in an `io::Error`.
fn classify_fetch_error(e: &reqwest::Error) -> PolicyFetch {
    if e.is_timeout() {
        return PolicyFetch::Timeout;
    }
    let mut innermost = e.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = source {
        if let Some(resolve) = err.downcast_ref::<ResolveFailure>() {
            return match resolve {
                ResolveFailure::NotFound(_) => PolicyFetch::NameNotFound,
                ResolveFailure::Timeout(_) => PolicyFetch::Timeout,
                ResolveFailure::ServFail { .. } => PolicyFetch::Failed {
                    detail: resolve.to_string(),
                },
                ResolveFailure::PrivateAddress { ip, .. } => PolicyFetch::PrivateAddress {
                    address: ip.to_string(),
                },
            };
        }
        if let Some(tls) = rustls_error(err) {
            return PolicyFetch::TlsFailed {
                detail: tls.to_string(),
            };
        }
        innermost = err.to_string();
        source = err.source();
    }
    if e.is_connect() {
        PolicyFetch::ConnectFailed
    } else {
        PolicyFetch::Failed { detail: innermost }
    }
}

/// The `rustls::Error` inside `err`, which the TLS connector wraps in one or
/// more `io::Error`s. (`io::Error::source` skips its own payload, so the
/// chain walk alone would miss it.)
fn rustls_error<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a rustls::Error> {
    let mut current = err;
    loop {
        if let Some(tls) = current.downcast_ref::<rustls::Error>() {
            return Some(tls);
        }
        let inner = current.downcast_ref::<std::io::Error>()?.get_ref()?;
        current = inner;
    }
}

/// An MTA-STS policy is a few lines; anything over this is not one.
const MAX_POLICY_BYTES: usize = 64 * 1024;

/// The response body as text, or `None` past [`MAX_POLICY_BYTES`] or on error.
async fn read_capped(mut resp: reqwest::Response) -> Option<String> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_POLICY_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).ok()
}

/// The policy fetch against a local TLS server: each [`PolicyFetch`] outcome
/// the classifier derives from reqwest's error chain, pinned so a dependency
/// upgrade that changes the chain fails here instead of degrading the labels.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{HttpResolve, TableResolver};
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const HOST: &str = "mta-sts.policy.test";

    /// A fixed host table in place of DNS; a missing host fails like NXDOMAIN.
    struct HostTable(HashMap<String, IpAddr>);

    impl reqwest::dns::Resolve for HostTable {
        fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            let found = self.0.get(name.as_str()).copied();
            let host = name.as_str().to_owned();
            Box::pin(async move {
                match found {
                    Some(ip) => {
                        let addrs: reqwest::dns::Addrs =
                            Box::new(std::iter::once(SocketAddr::new(ip, 0)));
                        Ok(addrs)
                    }
                    None => Err(Box::new(ResolveFailure::NotFound(host))
                        as Box<dyn std::error::Error + Send + Sync>),
                }
            })
        }
    }

    struct TestCert {
        cert: rcgen::Certificate,
        key: rcgen::KeyPair,
    }

    fn test_cert() -> TestCert {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec![HOST.to_owned()]).unwrap();
        TestCert {
            cert,
            key: signing_key,
        }
    }

    /// Serve `response` once over TLS on a free local port.
    async fn tls_server(cert: &TestCert, response: &'static str) -> SocketAddr {
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert.key.serialize_der().into()),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut tls) = acceptor.accept(tcp).await else {
                return;
            };
            let mut buf = [0u8; 4096];
            let _ = tls.read(&mut buf).await;
            let _ = tls.write_all(response.as_bytes()).await;
            let _ = tls.shutdown().await;
        });
        addr
    }

    fn checker(root: Option<&TestCert>, port: u16) -> DnsChecker {
        let table = HashMap::from([(HOST.to_owned(), IpAddr::from([127, 0, 0, 1]))]);
        let root = root.map(|c| reqwest::Certificate::from_der(c.cert.der()).unwrap());
        let http = http_client(Arc::new(HostTable(table)), root).unwrap();
        DnsChecker::from_parts(Arc::new(TableResolver::new()), http, port)
    }

    #[tokio::test]
    async fn policy_is_read_with_its_content_type() {
        let cert = test_cert();
        let addr = tls_server(
            &cert,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 20\r\nConnection: close\r\n\r\nversion: STSv1\nmode:",
        )
        .await;
        let (body, fetch) = checker(Some(&cert), addr.port())
            .fetch_policy("policy.test")
            .await;
        assert_eq!(
            fetch,
            PolicyFetch::Ok {
                content_type: Some("text/plain".into())
            }
        );
        assert_eq!(body.as_deref(), Some("version: STSv1\nmode:"));
    }

    #[tokio::test]
    async fn http_status_and_redirect_are_reported() {
        let cert = test_cert();
        let addr = tls_server(
            &cert,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (body, fetch) = checker(Some(&cert), addr.port())
            .fetch_policy("policy.test")
            .await;
        assert_eq!(fetch, PolicyFetch::HttpStatus { code: 404 });
        assert!(body.is_none());

        let addr = tls_server(
            &cert,
            "HTTP/1.1 301 Moved\r\nLocation: https://www.policy.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (_, fetch) = checker(Some(&cert), addr.port())
            .fetch_policy("policy.test")
            .await;
        assert_eq!(
            fetch,
            PolicyFetch::Redirect {
                location: Some("https://www.policy.test/".into())
            }
        );
    }

    /// A certificate the client does not trust (Fastmail's case: a certificate
    /// for another name) is a TLS failure naming the reason, not "no connection".
    #[tokio::test]
    async fn untrusted_certificate_is_a_tls_failure() {
        let cert = test_cert();
        let addr = tls_server(&cert, "HTTP/1.1 200 OK\r\n\r\n").await;
        let (_, fetch) = checker(None, addr.port()).fetch_policy("policy.test").await;
        let PolicyFetch::TlsFailed { detail } = fetch else {
            panic!("expected TlsFailed, got {fetch:?}");
        };
        assert!(detail.contains("certificate"), "{detail}");
    }

    /// The production client: resolving through the checks' [`Resolver`]
    /// (here a stub table) via [`HttpResolve`].
    fn checker_via_resolver(addresses: &[IpAddr], root: &TestCert, port: u16) -> DnsChecker {
        let dns: Arc<dyn crate::resolver::Resolver> =
            Arc::new(TableResolver::new().with_addresses(HOST, addresses));
        let root = reqwest::Certificate::from_der(root.cert.der()).unwrap();
        let http = http_client(Arc::new(HttpResolve(Arc::clone(&dns))), Some(root)).unwrap();
        DnsChecker::from_parts(dns, http, port)
    }

    /// A policy host on loopback is refused before any connection: the local
    /// server would serve a valid policy, and nothing reaches it.
    #[tokio::test]
    async fn a_host_on_a_private_address_is_never_fetched() {
        let cert = test_cert();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let loopback = IpAddr::from([127, 0, 0, 1]);
        let (body, fetch) = checker_via_resolver(&[loopback], &cert, port)
            .fetch_policy("policy.test")
            .await;
        assert_eq!(
            fetch,
            PolicyFetch::PrivateAddress {
                address: "127.0.0.1".into()
            }
        );
        assert!(body.is_none());
        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(
            accepted.is_err(),
            "the fetch connected to the private address"
        );

        // One internal address among public ones is enough to refuse.
        let mixed = [
            IpAddr::from([192, 0, 2, 10]),
            IpAddr::from([169, 254, 169, 254]),
        ];
        let (_, fetch) = checker_via_resolver(&mixed, &cert, port)
            .fetch_policy("policy.test")
            .await;
        assert_eq!(
            fetch,
            PolicyFetch::PrivateAddress {
                address: "169.254.169.254".into()
            }
        );
    }

    /// Public addresses go to the HTTP client unchanged: it connects to
    /// exactly these, with no lookup of its own.
    #[tokio::test]
    async fn public_addresses_are_handed_to_the_client_as_resolved() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;
        let public = [
            IpAddr::from([192, 0, 2, 10]),
            "2001:db8::10".parse::<IpAddr>().unwrap(),
        ];
        let resolve = HttpResolve(Arc::new(TableResolver::new().with_addresses(HOST, &public)));
        let addrs: Vec<IpAddr> = resolve
            .resolve(reqwest::dns::Name::from_str(HOST).unwrap())
            .await
            .unwrap()
            .map(|a| a.ip())
            .collect();
        assert_eq!(addrs, public);
    }

    #[tokio::test]
    async fn unresolvable_host_and_closed_port_are_told_apart() {
        let (_, fetch) = checker(None, 443).fetch_policy("other.test").await;
        assert_eq!(fetch, PolicyFetch::NameNotFound);

        // A port nothing listens on: bind to learn a free one, then release it.
        let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let (_, fetch) = checker(None, port).fetch_policy("policy.test").await;
        assert_eq!(fetch, PolicyFetch::ConnectFailed);
    }
}
