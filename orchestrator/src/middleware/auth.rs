// orchestrator/src/middleware/auth.rs
// Tower Layer: validates Ed25519-signed requests.
//
// Every protected request must include:
//   X-Pubkey:    hex Ed25519 verifying key (32 bytes = 64 hex chars)
//   X-Timestamp: Unix milliseconds (within ±30 s of server time)
//   X-Nonce:     hex 32-byte random nonce (single-use, stored for 60 s)
//   X-Signature: hex Ed25519 sig over sha256(body_bytes || timestamp_ms_str || nonce_hex)

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::state::SharedState;

/// Maximum allowed clock skew in milliseconds.
const MAX_CLOCK_SKEW_MS: i64 = 30_000;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn unauth(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg }))).into_response()
}

fn forbidden(msg: &str) -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "error": msg }))).into_response()
}

fn header_str(parts: &axum::http::request::Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
}

/// Core validation: timestamp window, signature over sha256(body||ts||nonce), nonce replay.
/// Returns the consumed body bytes on success so the handler can use them.
async fn validate_and_consume(
    state: &SharedState,
    parts: &axum::http::request::Parts,
    body_bytes: &[u8],
) -> Result<(), Response> {
    let pubkey_hex = header_str(parts, "X-Pubkey").ok_or_else(|| unauth("missing X-Pubkey"))?;
    let ts_str = header_str(parts, "X-Timestamp").ok_or_else(|| unauth("missing X-Timestamp"))?;
    let nonce_hex = header_str(parts, "X-Nonce").ok_or_else(|| unauth("missing X-Nonce"))?;
    let sig_hex = header_str(parts, "X-Signature").ok_or_else(|| unauth("missing X-Signature"))?;

    // 1. Timestamp window check
    let ts_ms: i64 = ts_str.parse().map_err(|_| unauth("invalid X-Timestamp"))?;
    let now_ms = Utc::now().timestamp_millis();
    if (now_ms - ts_ms).abs() > MAX_CLOCK_SKEW_MS {
        return Err(unauth("request expired"));
    }

    // 2. Validate nonce is 32 bytes (64 hex chars)
    if nonce_hex.len() != 64 || hex::decode(&nonce_hex).is_err() {
        return Err(unauth("invalid X-Nonce format"));
    }

    // 3. Compute sha256(body || timestamp || nonce) — must match what client signed
    let mut hasher = Sha256::new();
    hasher.update(body_bytes);
    hasher.update(ts_str.as_bytes());
    hasher.update(nonce_hex.as_bytes());
    let msg_hash = hasher.finalize();

    // 4. Verify Ed25519 signature over the hash
    if common::identity::verify_signature(&pubkey_hex, &msg_hash, &sig_hex).is_err() {
        return Err(unauth("invalid signature"));
    }

    // 5. Nonce replay prevention: attempt atomic insert; 0 rows = already used
    let inserted = sqlx::query!(
        r#"
        INSERT INTO request_nonces (nonce_hex, expires_at)
        VALUES ($1, NOW() + INTERVAL '60 seconds')
        ON CONFLICT (nonce_hex) DO NOTHING
        "#,
        nonce_hex,
    )
    .execute(&state.db)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "nonce store error"})),
        )
            .into_response()
    })?;

    if inserted.rows_affected() == 0 {
        return Err(unauth("nonce already used"));
    }

    Ok(())
}

/// Middleware: validates signature + timestamp + nonce for all API routes.
pub async fn verify_signed_request(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    let body_bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "body too large"})),
            )
                .into_response()
        }
    };

    if let Err(resp) = validate_and_consume(&state, &parts, &body_bytes).await {
        return resp;
    }

    next.run(Request::from_parts(parts, Body::from(body_bytes)))
        .await
}

/// Middleware: verify request is signed by the orchestrator admin key.
/// Applied only to `/admin/*` routes. Runs full signature + nonce check.
pub async fn require_admin_signature(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let pubkey_hex = match request
        .headers()
        .get("X-Pubkey")
        .and_then(|v| v.to_str().ok())
    {
        Some(v) => v.to_owned(),
        None => return unauth("missing X-Pubkey"),
    };

    // Must exactly match the admin signing key.
    let expected = state.admin_signing_key.pubkey_hex();
    if pubkey_hex != expected {
        return forbidden("admin key mismatch");
    }

    // Full validation (signature + nonce replay)
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "body too large"})),
            )
                .into_response()
        }
    };

    if let Err(resp) = validate_and_consume(&state, &parts, &body_bytes).await {
        return resp;
    }

    next.run(Request::from_parts(parts, Body::from(body_bytes)))
        .await
}
