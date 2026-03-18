pub mod auth;
pub mod config;
pub mod db;
pub mod discord;
pub mod web;
pub mod webhook;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, StatusCode},
    response::{IntoResponse, Response},
};
use config::AppConfig;
use db::DbPool;
use discord::DiscordTemplateEngine;
use reqwest::Client;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::{
    compression::CompressionLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

const DEFAULT_TRACING_FILTER: &str = "info,dmxforge=debug,tower_http=info";

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: DbPool,
    pub discord: DiscordTemplateEngine,
    pub http_client: Client,
    pub login_rate_limit: Arc<Mutex<HashMap<String, Vec<i64>>>>,
}

impl AppState {
    async fn bootstrap(config: AppConfig) -> Result<Arc<Self>> {
        let db = db::connect(&config).await?;
        let discord = DiscordTemplateEngine::new();
        let http_client = Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to build reqwest client")?;

        Ok(Arc::new(Self {
            config,
            db,
            discord,
            http_client,
            login_rate_limit: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
}

pub async fn run(config: AppConfig) -> Result<()> {
    let state = AppState::bootstrap(config.clone()).await?;
    let app = build_router(state);
    let listener = TcpListener::bind(config.socket_addr()).await?;

    tracing::info!(
        app_name = %config.app_name,
        address = %config.socket_addr(),
        "dmxforge listening"
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server exited unexpectedly")?;

    Ok(())
}

pub async fn migrate_only(config: AppConfig) -> Result<()> {
    let _state = AppState::bootstrap(config.clone()).await?;

    tracing::info!(
        app_name = %config.app_name,
        database_url = %config.database_url,
        "database migrations completed"
    );

    Ok(())
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let request_id = HeaderName::from_static("x-request-id");

    Router::new()
        .merge(web::router(state.clone()))
        .merge(webhook::router(state.clone()))
        .layer(DefaultBodyLimit::max(state.config.payload_limit_bytes()))
        .layer(CompressionLayer::new())
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
}

pub fn init_tracing() -> Result<()> {
    let env_filter = EnvFilter::new(DEFAULT_TRACING_FILTER);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .compact()
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing: {error}"))?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "internal server error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal server error".to_string(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = axum::Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}
