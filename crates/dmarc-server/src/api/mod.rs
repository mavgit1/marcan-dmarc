//! REST API route modules.

mod alerts;
mod analytics;
mod client_reports;
mod dns;
mod domain_detail;
mod domains;
mod export;
mod identity;
mod mapping;
mod meta;
mod organizations;
mod reporting_setup;
mod reports;
mod scope;
mod sender_names;
mod senders;
mod settings;
mod system;

pub(crate) use identity::identities_named;
pub use sender_names::load_sender_names;

use crate::AppState;
use axum::extract::Request;
use axum::http::{header, Method};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use std::sync::Arc;

/// Every POST must be JSON, or an image (the branding logo upload). A page on
/// another site can only send a POST as a form or a "simple" request (no JSON
/// or image content type without a CORS preflight, which this server never
/// allows), so this keeps other sites from changing settings or starting
/// connections with the visitor's login. PUT, PATCH and DELETE cannot be sent
/// cross-site at all.
async fn require_json_post(req: Request, next: Next) -> Response {
    let accepted = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            let v = v.trim_start().to_ascii_lowercase();
            v.starts_with("application/json") || v.starts_with("image/")
        });
    if req.method() == Method::POST && !accepted {
        return crate::error::AppError::unsupported_media_type(
            "POST requests must be application/json (or an image, for the logo)",
        )
        .into_response();
    }
    next.run(req).await
}

/// A method the path does not take: a JSON `405` like every other error.
async fn method_not_allowed() -> Response {
    crate::error::AppError::method_not_allowed("this path does not take that method")
        .into_response()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(system::healthz))
        .route("/api/system/health", get(system::get_system_health))
        .route("/api/meta", get(meta::get_meta))
        .route("/api/scope", get(scope::get_scope))
        .route("/api/statistics", get(analytics::get_statistics))
        .route("/api/timeseries", get(analytics::get_timeseries))
        .route("/api/alerts", get(alerts::list_alerts))
        .route("/api/reports", get(reports::list_reports))
        .route("/api/reports/{id}", get(reports::get_report))
        .route("/api/senders", get(senders::list_senders))
        .route(
            "/api/organizations",
            get(organizations::list_organizations).post(organizations::create_organization),
        )
        .route(
            "/api/organizations/{id}",
            get(organizations::get_organization)
                .patch(organizations::update_organization)
                .delete(organizations::delete_organization),
        )
        .route(
            "/api/organizations/{id}/client-report",
            get(organizations::get_client_report),
        )
        .route(
            "/api/domains",
            get(domains::list_domains).post(domains::create_domain),
        )
        .route(
            "/api/domains/attention",
            get(domains::list_attention_domains),
        )
        .route(
            "/api/domains/{id}",
            get(domain_detail::get_domain)
                .patch(domains::update_domain)
                .delete(domains::delete_domain),
        )
        .route(
            "/api/domains/{id}/failures",
            get(domain_detail::get_domain_failures),
        )
        .route("/api/senders/{ip}", get(senders::get_sender))
        .route("/api/export/domains.csv", get(export::export_domains_csv))
        .route("/api/export/senders.csv", get(export::export_senders_csv))
        .route(
            "/api/export/auth-issues.csv",
            get(export::export_auth_issues_csv),
        )
        .route(
            "/api/export/dns-findings.csv",
            get(export::export_dns_findings_csv),
        )
        .route(
            "/api/reporting-setup",
            get(reporting_setup::get_reporting_setup),
        )
        .route("/api/dns/check", get(dns::check_all))
        .route("/api/dns/dmarc", get(dns::check_dmarc))
        .route("/api/dns/spf", get(dns::check_spf))
        .route("/api/dns/dkim", get(dns::check_dkim))
        .route("/api/dns/mta-sts", get(dns::check_mta_sts))
        .route("/api/dns/tls-rpt", get(dns::check_tls_rpt))
        .route("/api/dns/bimi", get(dns::check_bimi))
        .route("/api/dns/ptr", get(dns::check_ptr))
        .route("/api/settings", get(settings::get_settings))
        .route("/api/settings/mailbox", put(settings::put_mailbox))
        .route("/api/settings/reporting", put(settings::put_reporting))
        .route("/api/settings/monitoring", put(settings::put_monitoring))
        .route("/api/settings/retention", put(settings::put_retention))
        .route("/api/settings/imap/connect", post(settings::connect_imap))
        .route("/api/settings/imap/sync", post(settings::sync_imap))
        .route("/api/settings/imap/reimport", post(settings::reimport_imap))
        .route("/api/settings/dns", put(settings::put_dns))
        .route("/api/settings/sending", put(settings::put_sending))
        .route("/api/settings/sending/test", post(settings::send_test))
        .route("/api/settings/alerts", put(settings::put_alerts))
        .route("/api/settings/alerts/test", post(settings::send_test_alert))
        .route(
            "/api/settings/client-reports",
            put(settings::put_client_reports),
        )
        .route(
            "/api/settings/client-reports/recipients",
            get(client_reports::get_recipients).put(client_reports::save_recipients),
        )
        .route(
            "/api/settings/client-reports/recipients/preview",
            post(client_reports::preview_recipients),
        )
        .route(
            "/api/settings/client-reports/email",
            get(settings::show_client_report_email),
        )
        .route(
            "/api/settings/client-reports/email/send",
            post(settings::send_client_report_email),
        )
        .route("/api/settings/license", put(settings::put_license))
        .route("/api/settings/branding", put(settings::put_branding))
        .route(
            "/api/settings/branding/logo",
            get(settings::show_logo)
                .post(settings::upload_logo)
                .delete(settings::delete_logo),
        )
        .route(
            "/api/settings/branding/preview",
            post(settings::preview_branding),
        )
        .route("/api/settings/onboarding", put(settings::put_onboarding))
        .route("/api/settings/updates", put(settings::put_updates))
        // The lists edited as text, one protocol each (`view::ListText`):
        // GET the list, POST …/preview a draft, PUT it to save.
        .route(
            "/api/settings/dkim-selectors",
            get(settings::get_dkim_selectors).put(settings::save_dkim_selectors),
        )
        .route(
            "/api/settings/dkim-selectors/preview",
            post(settings::preview_dkim_selectors),
        )
        .route(
            "/api/settings/sender-names",
            get(sender_names::get_sender_names).put(sender_names::save_sender_names),
        )
        .route(
            "/api/settings/sender-names/preview",
            post(sender_names::preview_sender_names),
        )
        .route(
            "/api/settings/domain-mapping",
            get(mapping::get_mapping).put(mapping::apply_mapping),
        )
        .route(
            "/api/settings/domain-mapping/preview",
            post(mapping::preview_mapping),
        )
        .method_not_allowed_fallback(method_not_allowed)
        .route_layer(middleware::from_fn(require_json_post))
        .with_state(state)
}
