// orchestrator/src/routes/tasks.rs
// POST /tasks/:task_id/result  — called by node-client once inference completes.
//
// Flow:
//   1. Look up node by pubkey (must be registered + not banned)
//   2. Load sub_task — guard against double-submit
//   3. Run the 5-step validation pipeline
//   4. Mark sub_task complete (with token counts)
//   5. Record reputation event
//   6. Calculate credit share and build+sign CreditReceipt
//   7. Persist receipt to DB
//   8. Check whether the parent job is now fully done
//   9. Return the signed receipt to the node

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    state::SharedState,
    task_manager::finalize_job_if_ready,
    validator::{validate_output, ValidationInput},
};
use common::types::{AgentRole, CreditReceipt, NodeTier};

// ---------------------------------------------------------------------------
// Request payload
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TaskResultRequest {
    pub output: String,
    /// sha256(prompt_shard || output) — hex-encoded
    pub proof_hash: String,
    /// Node's hex Ed25519 public key — must match X-Pubkey header
    pub node_pubkey: String,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn submit_task_result(
    State(state): State<SharedState>,
    Path(task_id): Path<Uuid>,
    Json(req): Json<TaskResultRequest>,
) -> AppResult<impl IntoResponse> {
    // 1. Resolve the node — must be registered + not banned
    let node = sqlx::query!(
        r#"
        SELECT id, tier, jobs_completed
        FROM nodes
        WHERE pubkey_hex = $1 AND is_banned = FALSE
        "#,
        req.node_pubkey
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("unknown or banned node".into()))?;

    // 2. Load the sub-task
    let task = sqlx::query!(
        r#"
        SELECT id, job_id, role, prompt_shard, status
        FROM sub_tasks
        WHERE id = $1
        "#,
        task_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    if task.status == "complete" {
        return Err(AppError::BadRequest("task already completed".into()));
    }

    // 3. Validate output through the pipeline
    let role: AgentRole =
        serde_json::from_str(&format!("\"{}\"", task.role)).unwrap_or(AgentRole::Coder);
    validate_output(ValidationInput {
        role: &role,
        prompt_shard_bytes: task.prompt_shard.as_bytes(),
        output: &req.output,
        node_proof_hash_hex: &req.proof_hash,
    })?;

    let tokens_in = req.tokens_in.unwrap_or(0);
    let tokens_out = req.tokens_out.unwrap_or(0);

    // 4. Mark sub-task complete
    sqlx::query!(
        r#"
        UPDATE sub_tasks
        SET status          = 'complete',
            result          = $2,
            assigned_node_id = $3,
            tokens_in       = $4,
            tokens_out      = $5,
            updated_at      = NOW(),
            completed_at    = NOW()
        WHERE id = $1
        "#,
        task_id,
        req.output,
        node.id,
        tokens_in as i32,
        tokens_out as i32,
    )
    .execute(&state.db)
    .await?;

    // 5. Reputation: small boost for completion
    sqlx::query!(
        r#"
        INSERT INTO reputation_events (node_id, event_type, delta, reason)
        VALUES ($1, 'task_complete', 0.01, $2)
        "#,
        node.id,
        format!("task {task_id} completed"),
    )
    .execute(&state.db)
    .await?;

    // 6. Calculate credit share across all completed sub-tasks in the parent job.
    let tier: NodeTier =
        serde_json::from_str(&format!("\"{}\"", node.tier)).unwrap_or(NodeTier::Edge);

    // Load all completed sub-tasks for the job to compute proportional share.
    let job_tasks = sqlx::query!(
        r#"
        SELECT role, tokens_in, tokens_out
        FROM sub_tasks
        WHERE job_id = $1 AND status = 'complete'
        "#,
        task.job_id
    )
    .fetch_all(&state.db)
    .await?;

    let all_tasks: Vec<(u32, u32, common::types::AgentRole, NodeTier)> = job_tasks
        .iter()
        .map(|t| {
            let r: common::types::AgentRole = serde_json::from_str(&format!("\"{}\"", t.role))
                .unwrap_or(common::types::AgentRole::Coder);
            let tin = t.tokens_in.unwrap_or(0) as u32;
            let tout = t.tokens_out.unwrap_or(0) as u32;
            // Use Edge as the tier for already-completed tasks (exact tier not stored per-task).
            (tin, tout, r, NodeTier::Edge)
        })
        .collect();

    // Derive total payout from the job's budget cap.
    let total_payout: f64 = sqlx::query_scalar!(
        "SELECT budget_cap_credits FROM jobs WHERE id = $1",
        task.job_id
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0.0);

    let credits = common::credits::node_credit_share(
        tokens_in,
        tokens_out,
        &role,
        &tier,
        &all_tasks,
        total_payout,
        node.jobs_completed as u64,
    );

    // 7. Build and sign CreditReceipt
    let now = Utc::now();
    let receipt_id = Uuid::new_v4();
    let nonce_hex = hex::encode(Uuid::new_v4().as_bytes());

    let mut receipt = CreditReceipt {
        id: receipt_id,
        job_id: task.job_id,
        sub_task_id: task_id,
        node_pubkey_hex: req.node_pubkey.clone(),
        credits,
        tokens_in,
        tokens_out,
        nonce_hex,
        issued_at: now,
        expires_at: now + chrono::Duration::hours(1),
        orchestrator_sig_hex: String::new(),
    };
    receipt.orchestrator_sig_hex = state.signing_key.sign_hex(&receipt.signable_bytes());

    // 8. Persist receipt
    sqlx::query!(
        r#"
        INSERT INTO credit_receipts
          (id, job_id, sub_task_id, node_pubkey_hex, credits,
           tokens_in, tokens_out, nonce_hex, issued_at, expires_at, orchestrator_sig_hex)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        receipt.id,
        receipt.job_id,
        receipt.sub_task_id,
        receipt.node_pubkey_hex,
        receipt.credits,
        receipt.tokens_in as i32,
        receipt.tokens_out as i32,
        receipt.nonce_hex,
        receipt.issued_at,
        receipt.expires_at,
        receipt.orchestrator_sig_hex,
    )
    .execute(&state.db)
    .await?;

    // 9. Check if all sub-tasks done → complete the parent job
    finalize_job_if_ready(&state, task.job_id).await?;

    Ok(Json(serde_json::json!({ "receipt": receipt })))
}
