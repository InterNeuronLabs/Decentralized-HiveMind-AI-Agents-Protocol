// orchestrator/src/routes/jobs.rs
// Job submission and status endpoints.
//
// POST /jobs/submit  — submit a new job
// GET  /jobs/:id     — poll job status

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use common::types::JobTier;
use secrecy::ExposeSecret;
use serde::Serialize;
use uuid::Uuid;
use validator::Validate;

use crate::{
    error::{AppError, AppResult},
    privacy,
    state::SharedState,
};

// ---------------------------------------------------------------------------
// Validated request wrapper
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, Validate)]
pub struct SubmitJobRequest {
    #[validate(length(min = 1, max = 32768))]
    pub prompt: String,
    /// Must be one of the allow-listed model slugs.
    #[validate(length(max = 64))]
    pub model_hint: Option<String>,
    #[validate(range(min = 0.01, max = 100_000.0))]
    pub budget_cap_credits: f64,
    pub tier: JobTier,
    #[validate(range(min = 10, max = 86400))]
    pub deadline_secs: u64,
    pub submitter_pubkey_hex: String,
}

/// Allow-listed model name slugs. Free-form model names are rejected to prevent injection.
const ALLOWED_MODELS: &[&str] = &[
    "llama3-8b",
    "llama3-70b",
    "mistral-7b",
    "phi-3-mini",
    "qwen2-7b",
    "deepseek-coder-7b",
];

// ---------------------------------------------------------------------------
// Submit
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SubmitJobResponse {
    pub job_id: Uuid,
    pub status: String,
}

pub async fn submit_job(
    State(state): State<SharedState>,
    Json(req): Json<SubmitJobRequest>,
) -> AppResult<impl IntoResponse> {
    // Input validation
    req.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    // Model allow-list check
    if let Some(ref model) = req.model_hint {
        if !ALLOWED_MODELS.contains(&model.as_str()) {
            return Err(AppError::BadRequest(format!(
                "model '{model}' not in allow-list"
            )));
        }
    }

    // Pubkey format validation
    if req.submitter_pubkey_hex.len() != 64 || hex::decode(&req.submitter_pubkey_hex).is_err() {
        return Err(AppError::BadRequest("invalid submitter_pubkey_hex".into()));
    }

    let deadline_at = Utc::now() + chrono::Duration::seconds(req.deadline_secs as i64);

    // PII tokenisation for Paid/Premium tiers
    let (final_prompt, _pii_map) = match req.tier {
        JobTier::Paid | JobTier::Premium => privacy::tokenise(&req.prompt),
        JobTier::Standard => (req.prompt.clone(), common::types::PiiMap::new()),
    };
    // _pii_map is stored in Arc<JobContext> in production; here it drops (no real jobs yet)

    // Encrypt prompt before storing in DB.
    // Using pgcrypto's pgp_sym_encrypt via a raw query.
    let encrypt_key = state.config.orchestrator_signing_key.expose_secret()[..32] // use first 32 hex chars as a deterministic key (replace with dedicated DB key in prod)
        .to_owned();

    let row = sqlx::query!(
        r#"
        INSERT INTO jobs
          (submitter_pubkey_hex, prompt_encrypted, model_hint, budget_cap_credits, tier, deadline_at)
        VALUES
          ($1, pgp_sym_encrypt($2, $3), $4, $5, $6, $7)
        RETURNING id
        "#,
        req.submitter_pubkey_hex,
        final_prompt,
        encrypt_key,
        req.model_hint,
        req.budget_cap_credits,
        format!("{:?}", req.tier),
        deadline_at,
    )
    .fetch_one(&state.db)
    .await?;

    // Seed sub-tasks appropriate for the job tier.
    // Each sub-task has a `position` field that the dispatcher uses for dependency ordering
    // (position N waits for all position < N tasks to be complete).
    //
    // Standard  → Planner(0) → Coder(1) → Summarizer(2)
    // Paid      → Planner(0) → Coder(1) → Critic(2)
    // Premium   → Planner(0) → Coder A(1) + Coder B(1 parallel) → Critic(2) → Aggregator(3)
    let min_tier = match req.tier {
        JobTier::Premium => "Pro",
        _ => "Edge",
    };

    // (role_str, position, agent_index)
    let subtask_plan: Vec<(&str, i32, i32)> = match req.tier {
        JobTier::Standard => vec![("Planner", 0, 0), ("Coder", 1, 0), ("Summarizer", 2, 0)],
        JobTier::Paid => vec![("Planner", 0, 0), ("Coder", 1, 0), ("Critic", 2, 0)],
        JobTier::Premium => vec![
            ("Planner", 0, 0),
            ("Coder", 1, 0), // parallel
            ("Coder", 1, 1), // parallel
            ("Critic", 2, 0),
            ("Aggregator", 3, 0),
        ],
    };

    for (role, position, agent_index) in subtask_plan {
        sqlx::query!(
            r#"
            INSERT INTO sub_tasks (job_id, role, prompt_shard, min_tier, position, agent_index)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            row.id,
            role,
            final_prompt,
            min_tier,
            position,
            agent_index,
        )
        .execute(&state.db)
        .await?;
    }

    tracing::info!(job_id = %row.id, tier = ?req.tier, "job submitted");

    Ok((
        StatusCode::CREATED,
        Json(SubmitJobResponse {
            job_id: row.id,
            status: "pending".into(),
        }),
    ))
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct JobStatusResponse {
    pub job_id: Uuid,
    pub status: String,
    pub total_cost_credits: Option<f64>,
    pub created_at: chrono::DateTime<Utc>,
    pub completed_at: Option<chrono::DateTime<Utc>>,
}

pub async fn job_status(
    State(state): State<SharedState>,
    Path(job_id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    let row = sqlx::query!(
        "SELECT id, status, total_cost_credits, created_at, completed_at FROM jobs WHERE id = $1",
        job_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(JobStatusResponse {
        job_id: row.id,
        status: row.status,
        total_cost_credits: row.total_cost_credits,
        created_at: row.created_at,
        completed_at: row.completed_at,
    }))
}

// ---------------------------------------------------------------------------
// List — returns up to 50 most recent jobs for the authenticated submitter.
// The caller's identity comes from X-Pubkey (already verified by auth middleware).
// ---------------------------------------------------------------------------

pub async fn list_jobs(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let pubkey_hex = headers
        .get("X-Pubkey")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("missing X-Pubkey header".into()))?
        .to_owned();

    // Pubkey is already signature-verified by the middleware; just sanity-check format.
    if pubkey_hex.len() != 64 || hex::decode(&pubkey_hex).is_err() {
        return Err(AppError::BadRequest("invalid X-Pubkey format".into()));
    }

    let rows = sqlx::query!(
        r#"
        SELECT id, status, total_cost_credits, created_at, completed_at
        FROM jobs
        WHERE submitter_pubkey_hex = $1
        ORDER BY created_at DESC
        LIMIT 50
        "#,
        pubkey_hex,
    )
    .fetch_all(&state.db)
    .await?;

    let jobs: Vec<JobStatusResponse> = rows
        .into_iter()
        .map(|r| JobStatusResponse {
            job_id: r.id,
            status: r.status,
            total_cost_credits: r.total_cost_credits,
            created_at: r.created_at,
            completed_at: r.completed_at,
        })
        .collect();

    Ok(Json(jobs))
}
