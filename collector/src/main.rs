mod alerts;
mod auth;
mod config;
mod connectivity;
mod db;
mod error;
mod events;
mod health;
mod models;
mod query;
mod ratelimit;
mod reboot;
mod routes;
mod state;
mod validation;

use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::routes::{build_router, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("configuracion invalida: {e}");
            std::process::exit(2);
        }
    };

    /* axum-server requiere que el crypto provider de rustls se instale
     * explicitamente antes de construir el ServerConfig. Es idempotente. */
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let event_bus = events::EventBus::new(config.ws_channel_capacity);

    let pool = match db::connect(&config.database_url, config.db_max_connections).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("no se pudo conectar a postgres: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) = db::run_migrations(&pool).await {
        tracing::error!("fallo la migracion de esquema: {e}");
        std::process::exit(2);
    }

    let listener = match tokio::net::TcpListener::bind(&config.listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(
                "no se pudo bindear el listener en {}: {e}",
                config.listen_addr
            );
            std::process::exit(2);
        }
    };

    let rate_limit_enabled = config.rate_limit_enabled;
    let rate_limit_rate = config.rate_limit_rate;
    let rate_limit_burst = config.rate_limit_burst;
    let listen_addr = config.listen_addr.clone();
    let tls_cert = config.tls_cert.clone();
    let tls_key = config.tls_key.clone();

    let state = AppState {
        pool,
        config: Arc::new(config),
        events: event_bus.clone(),
        limiter: Arc::new(ratelimit::RateLimiter::new(ratelimit::RatePolicy {
            rate_per_sec: if rate_limit_enabled {
                rate_limit_rate
            } else {
                0.0
            },
            burst: rate_limit_burst,
        })),
    };
    alerts::spawn_evaluator(
        state.pool.clone(),
        Arc::clone(&state.config),
        event_bus.clone(),
    );
    health::spawn_health_runner(
        state.pool.clone(),
        Arc::clone(&state.config),
        event_bus.clone(),
    );
    connectivity::spawn_connectivity_runner(
        state.pool.clone(),
        Arc::clone(&state.config),
        event_bus,
    );
    let app = build_router(state);
    let make_service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();

    match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => {
            let tls_config =
                match axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(
                            "no se pudo cargar el certificado TLS (OBS_TLS_CERT / OBS_TLS_KEY): {e}"
                        );
                        std::process::exit(2);
                    }
                };
            let tcp_listener = match listener.into_std() {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("no se pudo convertir el listener a TLS: {e}");
                    std::process::exit(2);
                }
            };
            let server = match axum_server::from_tcp_rustls(tcp_listener, tls_config) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("no se pudo preparar el listener TLS: {e}");
                    std::process::exit(2);
                }
            };
            let handle = axum_server::Handle::new();
            let server = server.handle(handle.clone());
            tokio::spawn(async move {
                shutdown_signal().await;
                handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
            });
            tracing::info!("collector listening on https://{listen_addr}");
            server.serve(make_service).await.expect("server error");
        }
        (None, None) => {
            tracing::info!("collector listening on {}", listener.local_addr().unwrap());
            axum::serve(listener, make_service)
                .with_graceful_shutdown(shutdown_signal())
                .await
                .expect("server error");
        }
        _ => {
            /* Caso imposible: validate_tls_pair falla ruidoso antes. */
            tracing::error!(
                "configuracion TLS invalida: OBS_TLS_CERT y OBS_TLS_KEY deben ir juntos"
            );
            std::process::exit(2);
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal recibido");
}
