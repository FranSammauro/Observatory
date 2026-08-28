mod auth;
mod config;
mod db;
mod error;
mod models;
mod routes;
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

    let state = AppState {
        pool,
        config: Arc::new(config),
    };
    let app = build_router(state);

    tracing::info!("collector listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
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
