// orchestrator/src/routes/admin.rs
// Owner-only admin endpoints. Auth requires a signature from ADMIN_SIGNING_KEY.
// This route is mounted at /admin/* and intentionally not documented in the public API.
//
// POST /admin/nodes/ban     — ban a node
// POST /admin/nodes/unban   — unban a node
// GET  /admin/nodes/flagged — list nodes in manual review

use axum::{extract::State, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    state::SharedState,
};

// ---------------------------------------------------------------------------
// Ban
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BanRequest {
    pub node_id: Uuid,
    pub reason: String,
    /// Optional: Unix timestamp when the ban expires. None = permanent.
    pub ban_expires_at: Option<DateTime<Utc>>,
}

pub async fn ban_node(
    State(state): State<SharedState>,
    Json(req): Json<BanRequest>,
) -> AppResult<impl IntoResponse> {
    if req.reason.trim().is_empty() {
        return Err(AppError::BadRequest("ban reason is required".into()));
    }

    let rows = sqlx::query!(
        r#"
        UPDATE nodes
        SET is_banned = TRUE, ban_reason = $2, ban_expires_at = $3
        WHERE id = $1
        "#,
        req.node_id,
        req.reason,
        req.ban_expires_at,
    )
    .execute(&state.db)
    .await?;

    if rows.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    // Append an immutable reputation event
    sqlx::query!(
        r#"
        INSERT INTO reputation_events (node_id, event_type, delta, reason)
        VALUES ($1, 'ban', -1.0, $2)
        "#,
        req.node_id,
        req.reason,
    )
    .execute(&state.db)
    .await?;

    tracing::warn!(node_id = %req.node_id, reason = %req.reason, "node banned by admin");

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Unban
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UnbanRequest {
    pub node_id: Uuid,
}

pub async fn unban_node(
    State(state): State<SharedState>,
    Json(req): Json<UnbanRequest>,
) -> AppResult<impl IntoResponse> {
    sqlx::query!(
        "UPDATE nodes SET is_banned = FALSE, ban_reason = NULL, ban_expires_at = NULL WHERE id = $1",
        req.node_id
    )
    .execute(&state.db)
    .await?;

    tracing::info!(node_id = %req.node_id, "node unbanned by admin");
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Flagged nodes
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FlaggedNode {
    pub id: Uuid,
    pub pubkey_hex: String,
    pub tier: String,
    pub reputation_score: f64,
    pub is_banned: bool,
    pub ban_reason: Option<String>,
}

pub async fn flagged_nodes(State(state): State<SharedState>) -> AppResult<impl IntoResponse> {
    let rows = sqlx::query!(
        r#"
        SELECT id, pubkey_hex, tier, reputation_score, is_banned, ban_reason
        FROM nodes
        WHERE reputation_score < 0.3 OR is_banned = TRUE
        ORDER BY reputation_score ASC
        LIMIT 200
        "#
    )
    .fetch_all(&state.db)
    .await?;

    let flagged: Vec<FlaggedNode> = rows
        .into_iter()
        .map(|r| FlaggedNode {
            id: r.id,
            pubkey_hex: r.pubkey_hex,
            tier: r.tier,
            reputation_score: r.reputation_score,
            is_banned: r.is_banned,
            ban_reason: r.ban_reason,
        })
        .collect();

    Ok(Json(flagged))
}
