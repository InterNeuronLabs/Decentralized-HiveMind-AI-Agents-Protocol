// orchestrator/src/main.rs

use std::net::SocketAddr;

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod dag;
mod error;
mod middleware;
mod privacy;
mod routes;
mod state;
mod task_manager;
mod validator;

use std::sync::Arc;

use config::AppConfig;
use state::{AppState, SharedState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (dev convenience)
    let _ = dotenvy::dotenv();

    // Tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "orchestrator=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    let shared: SharedState = Arc::new(AppState::new(config).await?);

    // Spawn background task dispatcher
    task_manager::spawn_task_manager(shared.clone());

    // Admin sub-router – signed by admin key only
    let admin_router = Router::new()
        .route("/nodes/ban", post(routes::admin::ban_node))
        .route("/nodes/unban", post(routes::admin::unban_node))
        .route("/nodes/flagged", get(routes::admin::flagged_nodes))
        .layer(axum::middleware::from_fn_with_state(
            shared.clone(),
            crate::middleware::auth::require_admin_signature,
        ));

    // Protected routes – require valid Ed25519 signature
    let protected = Router::new()
        .route("/nodes/register", post(routes::nodes::register_node))
        .route("/nodes/heartbeat", post(routes::nodes::heartbeat))
        .route("/nodes", get(routes::nodes::list_nodes))
        .route("/jobs", get(routes::jobs::list_jobs))
        .route("/jobs/submit", post(routes::jobs::submit_job))
        .route("/jobs/:id", get(routes::jobs::job_status))
        .route("/tasks/:id/result", post(routes::tasks::submit_task_result))
        .nest("/admin", admin_router)
        .layer(axum::middleware::from_fn_with_state(
            shared.clone(),
            crate::middleware::auth::verify_signed_request,
        ));

    // Full application: unauthenticated health check + all protected routes
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .with_state(shared.clone());

    let port = shared.config.port;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("orchestrator listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Lightweight liveness probe. Does not require authentication.
async fn health_check(State(state): State<SharedState>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db).await.is_ok();
    let status = if db_ok { "ok" } else { "degraded" };
    Json(serde_json::json!({ "status": status, "db": if db_ok { "ok" } else { "error" } }))
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    tracing::info!("shutdown signal received");
}
