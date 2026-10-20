//! Marcan HTTP server binary.

use anyhow::Context;
use clap::Parser;
use dmarc_server::{router_with_state, AllowedHosts, AppState, Background};
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// File name of the SQLite database inside `--data-dir`.
const DB_FILE: &str = "marcan.db";

/// How long shutdown waits for background work in hand before aborting it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
#[command(name = "dmarc-server", about = "Marcan REST API + IMAP fetch loop")]
struct Config {
    /// Listen address (host:port).
    #[arg(long = "bind", env = "BIND_ADDR", default_value = "127.0.0.1:8080")]
    bind_addr: String,

    /// Directory holding the SQLite database (`marcan.db`); created if missing.
    #[arg(long, env = "DATA_DIR", default_value = ".")]
    data_dir: PathBuf,

    /// Host names requests may be addressed to besides the bind address and
    /// localhost, comma-separated (the name a proxy forwards). Bound to every
    /// interface (0.0.0.0) and without names, any host is answered.
    #[arg(long = "allowed-hosts", env = "ALLOWED_HOSTS", value_delimiter = ',')]
    allowed_hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::parse();
    let db_path = cfg.data_dir.join(DB_FILE);
    info!(path = %db_path.display(), "initializing database");
    let pool = dmarc_store::init(&db_path)
        .await
        .with_context(|| format!("failed to initialize database {}", db_path.display()))?;

    let names = dmarc_server::api::load_sender_names(&pool)
        .await
        .context("failed to load sender name overrides")?;
    let allowed_hosts = AllowedHosts::for_bind(&cfg.bind_addr, &cfg.allowed_hosts);
    info!(?allowed_hosts, "answering requests for these hosts");
    let demo_dns = dmarc_server::demo_resolver(&pool)
        .await
        .context("failed to read the demo DNS records")?;
    let mut state = AppState::new(pool)
        .with_sender_names(names)
        .with_allowed_hosts(allowed_hosts);
    if let Some(resolver) = demo_dns {
        tracing::warn!("demo database: DNS checks answer from its made-up records, not from DNS");
        state = state.with_dns_resolver(resolver);
    }
    let background = Background::start(&state);
    let app = router_with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", cfg.bind_addr))?;
    info!(bind_addr = %cfg.bind_addr, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    background.shutdown(SHUTDOWN_GRACE).await;
    info!("shut down");
    Ok(())
}

/// Resolves on Ctrl-C or (on Unix) SIGTERM, which is what `docker stop` sends.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    info!("shutdown signal received, stopping");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_hosts_are_a_comma_list() {
        let cfg = Config::try_parse_from(["dmarc-server"]).unwrap();
        assert!(cfg.allowed_hosts.is_empty());
        let cfg = Config::try_parse_from([
            "dmarc-server",
            "--allowed-hosts",
            "dmarc.example.com,marcan.example.net",
        ])
        .unwrap();
        assert_eq!(
            cfg.allowed_hosts,
            ["dmarc.example.com", "marcan.example.net"]
        );
    }
}
