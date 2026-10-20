//! What every request and response passes through before the routes: the
//! `Host` check against DNS rebinding, and the headers that keep the UI out
//! of other sites' frames.

use crate::error::AppError;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dmarc_domain::normalize_name;
use std::net::IpAddr;
use std::sync::Arc;

/// Names every restricted server answers to, whatever it is bound to.
const LOOPBACK_NAMES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The `Host` names the server answers to.
///
/// Bound to one address (the binary's default `127.0.0.1:8080`), it answers
/// only to that address, `localhost` and the names given with
/// `--allowed-hosts`. A page on another site that points its own name at
/// 127.0.0.1 (DNS rebinding) still sends its own name as the `Host`, and is
/// refused. Bound to every interface (the image's `0.0.0.0:8080`, behind a
/// proxy that forwards the public name) it answers to any host, unless names
/// are given: then to those and the loopback names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedHosts {
    /// `None`: any host.
    names: Option<Vec<String>>,
}

impl AllowedHosts {
    /// Any `Host` (tests, and a server bound to every interface).
    pub fn any() -> Self {
        Self::default()
    }

    /// The hosts for a server listening on `bind_addr` (`host:port`), plus
    /// `extra` (the `--allowed-hosts` names; a port on them is ignored).
    pub fn for_bind(bind_addr: &str, extra: &[String]) -> Self {
        let bind_host = host_name(bind_addr);
        let everywhere = bind_host
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_unspecified());
        let extra: Vec<String> = extra
            .iter()
            .map(|name| host_name(name))
            .filter(|name| !name.is_empty())
            .collect();
        if everywhere && extra.is_empty() {
            return Self::any();
        }
        let mut names: Vec<String> = LOOPBACK_NAMES.iter().map(|n| (*n).to_owned()).collect();
        if !everywhere {
            names.push(bind_host);
        }
        names.extend(extra);
        names.sort();
        names.dedup();
        Self { names: Some(names) }
    }

    /// Whether a request whose `Host` (or HTTP/2 authority) is `host` is
    /// answered; a request without one only when any host is.
    pub fn allows(&self, host: Option<&str>) -> bool {
        match (&self.names, host) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(names), Some(host)) => {
                let name = host_name(host);
                names.contains(&name)
            }
        }
    }
}

/// The host of `authority` (`host`, `host:port`, `[v6]:port`, `v6`),
/// lowercased, without brackets, port or trailing dot.
fn host_name(authority: &str) -> String {
    let a = authority.trim();
    let host = if let Some(rest) = a.strip_prefix('[') {
        rest.split_once(']').map_or(rest, |(inside, _)| inside)
    } else if a.matches(':').count() == 1 {
        a.split_once(':').map_or(a, |(host, _)| host)
    } else {
        // No port, or a bare IPv6 address.
        a
    };
    normalize_name(host)
}

/// Refuse a request whose `Host` is not one this server answers to.
pub(crate) async fn check_host(
    State(allowed): State<Arc<AllowedHosts>>,
    req: Request,
    next: Next,
) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            req.uri()
                .authority()
                .map(axum::http::uri::Authority::as_str)
        });
    if !allowed.allows(host) {
        let shown = host.unwrap_or("(none)");
        tracing::warn!(
            host = shown,
            "refused a request for a host this server does not answer to"
        );
        return AppError::bad_request(format!(
            "This server does not answer to the host {shown}. Add it with --allowed-hosts (ALLOWED_HOSTS)."
        ))
        .into_response();
    }
    next.run(req).await
}

/// No page may show the UI in a frame (clickjacking with a login the browser
/// remembers), whatever proxy is in front.
pub(crate) async fn frame_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn host_names_lose_port_brackets_case_and_dot() {
        assert_eq!(host_name("Example.COM:8080"), "example.com");
        assert_eq!(host_name("dmarc.example.com."), "dmarc.example.com");
        assert_eq!(host_name("[::1]:8080"), "::1");
        assert_eq!(host_name("::1"), "::1");
        assert_eq!(host_name("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn a_loopback_bind_answers_to_itself_localhost_and_the_list() {
        let allowed = AllowedHosts::for_bind("127.0.0.1:8080", &list(&["Dmarc.Example.com"]));
        for host in [
            "127.0.0.1:8080",
            "localhost:8080",
            "LOCALHOST",
            "[::1]:8080",
            "dmarc.example.com",
            "dmarc.example.com:443",
        ] {
            assert!(allowed.allows(Some(host)), "{host}");
        }
        for host in ["attacker.example:8080", "example.com", "127.0.0.2:8080", ""] {
            assert!(!allowed.allows(Some(host)), "{host}");
        }
        assert!(!allowed.allows(None));
    }

    #[test]
    fn a_bind_on_one_address_answers_to_that_address() {
        let allowed = AllowedHosts::for_bind("192.0.2.5:8080", &[]);
        assert!(allowed.allows(Some("192.0.2.5:8080")));
        assert!(allowed.allows(Some("localhost")));
        assert!(!allowed.allows(Some("rebind.example")));
        let v6 = AllowedHosts::for_bind("[2001:db8::5]:8080", &[]);
        assert!(v6.allows(Some("[2001:db8::5]:8080")));
    }

    #[test]
    fn every_interface_answers_to_any_host_unless_names_are_given() {
        for bind in ["0.0.0.0:8080", "[::]:8080"] {
            let any = AllowedHosts::for_bind(bind, &[]);
            assert_eq!(any, AllowedHosts::any());
            assert!(any.allows(Some("dmarc.example.com")));
            assert!(any.allows(None));
            // Blank entries (ALLOWED_HOSTS="") name nothing.
            assert_eq!(
                AllowedHosts::for_bind(bind, &list(&[" "])),
                AllowedHosts::any()
            );

            let named = AllowedHosts::for_bind(bind, &list(&["dmarc.example.com"]));
            assert!(named.allows(Some("dmarc.example.com")));
            assert!(named.allows(Some("localhost:8080")));
            assert!(!named.allows(Some("0.0.0.0:8080")));
            assert!(!named.allows(Some("other.example")));
        }
    }
}
