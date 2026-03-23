// orchestrator/src/routes/nodes.rs
// Node registration and heartbeat endpoints.
//
// POST /nodes/register — new node joins the network
// POST /nodes/heartbeat — keep-alive + capability update
// GET  /nodes           — list active nodes (public, for monitoring)

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use common::types::NodeCapabilities;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    state::SharedState,
};

// ---------------------------------------------------------------------------
// Register
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// Ed25519 verifying key hex (64 hex chars = 32 bytes).
    pub pubkey_hex: String,
    pub capabilities: NodeCapabilities,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub node_id: Uuid,
    pub message: String,
}

pub async fn register_node(
    State(state): State<SharedState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<impl IntoResponse> {
    // Validate pubkey length (32 bytes = 64 hex chars)
    if req.pubkey_hex.len() != 64 || hex::decode(&req.pubkey_hex).is_err() {
        return Err(AppError::BadRequest("invalid pubkey_hex".into()));
    }

    // Upsert node — if pubkey already exists, update capabilities + last_seen
    let row = sqlx::query!(
        r#"
        INSERT INTO nodes (pubkey_hex, capabilities, tier, last_seen_at)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (pubkey_hex) DO UPDATE
          SET capabilities = EXCLUDED.capabilities,
              tier         = EXCLUDED.tier,
              last_seen_at = NOW()
        RETURNING id
        "#,
        req.pubkey_hex,
        serde_json::to_value(&req.capabilities).unwrap(),
        format!("{:?}", req.capabilities.tier),
    )
    .fetch_one(&state.db)
    .await?;

    tracing::info!(
        node_id = %row.id,
        pubkey  = %req.pubkey_hex,
        tier    = ?req.capabilities.tier,
        "node registered"
    );

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            node_id: row.id,
            message: "registered".into(),
        }),
    ))
}

// ---------------------------------------------------------------------------
// Heartbeat
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub node_id: Uuid,
}

pub async fn heartbeat(
    State(state): State<SharedState>,
    Json(req): Json<HeartbeatRequest>,
) -> AppResult<impl IntoResponse> {
    let updated = sqlx::query!(
        "UPDATE nodes SET last_seen_at = NOW() WHERE id = $1 AND is_banned = FALSE",
        req.node_id
    )
    .execute(&state.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// List active nodes (public monitoring endpoint, no auth required)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct NodeSummary {
    pub id: Uuid,
    pub tier: String,
    pub reputation_score: f64,
    pub jobs_completed: i64,
    pub last_seen_at: chrono::DateTime<Utc>,
}

pub async fn list_nodes(State(state): State<SharedState>) -> AppResult<impl IntoResponse> {
    let rows = sqlx::query!(
        r#"
        SELECT id, tier, reputation_score, jobs_completed, last_seen_at
        FROM nodes
        WHERE is_banned = FALSE
          AND last_seen_at > NOW() - INTERVAL '5 minutes'
        ORDER BY reputation_score DESC
        LIMIT 100
        "#
    )
    .fetch_all(&state.db)
    .await?;

    let summaries: Vec<NodeSummary> = rows
        .into_iter()
        .map(|r| NodeSummary {
            id: r.id,
            tier: r.tier,
            reputation_score: r.reputation_score,
            jobs_completed: r.jobs_completed,
            last_seen_at: r.last_seen_at,
        })
        .collect();

    Ok(Json(summaries))
}
