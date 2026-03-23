// orchestrator/src/middleware/rate_limit.rs
// Per-wallet-pubkey and per-IP rate limiting using `governor`.
#![allow(dead_code)]

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use governor::{
    clock::{Clock, DefaultClock},
    state::InMemoryState,
    Quota, RateLimiter,
};
use serde_json::json;
use std::{net::SocketAddr, num::NonZeroU32, sync::Arc, time::Duration};

type KeyedLimiter = Arc<RateLimiter<String, dashmap::DashMap<String, InMemoryState>, DefaultClock>>;

/// Create a keyed rate limiter: `quota` requests per `period` per unique key.
pub fn keyed_limiter(per_period: NonZeroU32, period: Duration) -> KeyedLimiter {
    Arc::new(RateLimiter::keyed(
        Quota::with_period(period / per_period.get()).unwrap(),
    ))
}

/// Middleware: limit job submissions to 10/minute per wallet pubkey.
pub async fn job_submit_limiter(
    limiter: axum::extract::Extension<KeyedLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let pubkey = request
        .headers()
        .get("X-Pubkey")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    match limiter.check_key(&pubkey) {
        Ok(_) => next.run(request).await,
        Err(negative) => {
            let retry_after =
                negative.wait_time_from(governor::clock::DefaultClock::default().now());
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "rate limited" })),
            )
                .into_response();
            response.headers_mut().insert(
                "Retry-After",
                retry_after.as_secs().to_string().parse().unwrap(),
            );
            response
        }
    }
}

/// Middleware: limit registration to 5/minute per IP.
pub async fn registration_ip_limiter(
    limiter: axum::extract::Extension<KeyedLimiter>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();
    match limiter.check_key(&ip) {
        Ok(_) => next.run(request).await,
        Err(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "rate limited" })),
        )
            .into_response(),
    }
}
