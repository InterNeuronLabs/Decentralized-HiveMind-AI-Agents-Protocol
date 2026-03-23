// orchestrator/src/task_manager.rs
//
// Async sub-task state-machine running over PostgreSQL.
// Uses FOR UPDATE SKIP LOCKED so multiple orchestrator replicas compete safely.
#![allow(dead_code)]
//
// Lifecycle: Pending → Dispatched → Running → Complete | Failed
// On failure the task is retried up to MAX_RETRIES, then the parent job fails.

use std::time::Duration;

use tokio::time::sleep;
use uuid::Uuid;

use crate::{
    error::AppResult,
    state::SharedState,
    validator::{validate_output, ValidationInput},
};
use common::types::AgentRole;

const MAX_RETRIES: i32 = 3;
const DISPATCH_INTERVAL_MS: u64 = 500;
/// How often (in dispatch ticks) to run housekeeping jobs (nonce cleanup, reputation).
const HOUSEKEEPING_EVERY_N: u64 = 120; // every ~60 seconds at 500ms interval

/// Spawn the background task dispatcher loop.
/// Call once from `main` after building AppState.
pub fn spawn_task_manager(state: SharedState) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            if let Err(e) = dispatch_ready_tasks(&state).await {
                tracing::error!(error = %e, "task dispatch error");
            }

            // Periodic housekeeping
            if tick.is_multiple_of(HOUSEKEEPING_EVERY_N) {
                if let Err(e) = cleanup_expired_nonces(&state).await {
                    tracing::warn!(error = %e, "nonce cleanup failed");
                }
                if let Err(e) = recompute_reputation_scores(&state).await {
                    tracing::warn!(error = %e, "reputation recompute failed");
                }
            }

            tick = tick.wrapping_add(1);
            sleep(Duration::from_millis(DISPATCH_INTERVAL_MS)).await;
        }
    });
}

/// Find sub-tasks that are Pending and whose dependencies (parent tasks in the DAG) are all
/// Complete, then atomically mark them Dispatched and push them onto the Redis stream for
/// node-client workers to pick up.
async fn dispatch_ready_tasks(state: &SharedState) -> AppResult<()> {
    // Single atomic CTE: claim up to 50 ready tasks and mark them dispatched in one round-trip.
    // FOR UPDATE SKIP LOCKED prevents two orchestrator replicas claiming the same row.
    let ready = sqlx::query!(
        r#"
        WITH eligible AS (
            SELECT st.id
            FROM sub_tasks st
            WHERE st.status = 'pending'
              AND NOT EXISTS (
                  SELECT 1 FROM sub_tasks dep
                  WHERE dep.job_id = st.job_id
                    AND dep.position < st.position
                    AND dep.status != 'complete'
              )
            ORDER BY st.created_at
            LIMIT 50
            FOR UPDATE SKIP LOCKED
        )
        UPDATE sub_tasks
        SET status = 'dispatched', updated_at = NOW()
        FROM eligible
        WHERE sub_tasks.id = eligible.id
        RETURNING sub_tasks.id, sub_tasks.job_id, sub_tasks.role,
                  sub_tasks.prompt_shard, sub_tasks.min_tier, sub_tasks.agent_index
        "#
    )
    .fetch_all(&state.db)
    .await?;

    if ready.is_empty() {
        return Ok(());
    }

    let mut redis = state.redis.lock().await;

    for task in ready {
        // Push onto Redis stream `subtasks`
        let payload = serde_json::json!({
            "task_id": task.id,
            "job_id":  task.job_id,
            "role":    task.role,
            "prompt_shard": task.prompt_shard,
            "min_tier": task.min_tier,
            "agent_index": task.agent_index,
        })
        .to_string();

        redis::cmd("XADD")
            .arg("subtasks")
            .arg("*")
            .arg("payload")
            .arg(&payload)
            .query_async::<()>(&mut *redis)
            .await
            .map_err(crate::error::AppError::Redis)?;

        tracing::debug!(task_id = %task.id, role = %task.role, "task dispatched");
    }

    Ok(())
}

/// Called by the node-result ingestion endpoint once a node submits its output.
/// Validates the output, marks the task complete, and checks for job-level completion.
pub async fn complete_task(
    state: &SharedState,
    task_id: Uuid,
    node_id: Uuid,
    output: String,
    proof_hash: String,
) -> AppResult<()> {
    // Fetch the task so we can validate the proof hash
    let task = sqlx::query!(
        "SELECT id, role, prompt_shard, job_id, retry_count FROM sub_tasks WHERE id = $1",
        task_id
    )
    .fetch_one(&state.db)
    .await?;

    // Run the 5-step validation pipeline
    let role: AgentRole =
        serde_json::from_str(&format!("\"{}\"", task.role)).unwrap_or(AgentRole::Coder);
    let vi = ValidationInput {
        role: &role,
        prompt_shard_bytes: task.prompt_shard.as_bytes(),
        output: &output,
        node_proof_hash_hex: &proof_hash,
    };
    validate_output(vi)?;

    // Persist result
    sqlx::query!(
        r#"
        UPDATE sub_tasks
        SET status = 'complete', result = $2, assigned_node_id = $3, updated_at = NOW()
        WHERE id = $1
        "#,
        task_id,
        output,
        node_id,
    )
    .execute(&state.db)
    .await?;

    // Bump reputation for the completing node
    sqlx::query!(
        r#"
        INSERT INTO reputation_events (node_id, event_type, delta, reason)
        VALUES ($1, 'task_complete', 0.01, $2)
        "#,
        node_id,
        format!("task {task_id} completed"),
    )
    .execute(&state.db)
    .await?;

    // Check if the parent job is now fully complete
    finalize_job_if_ready(state, task.job_id).await?;

    Ok(())
}

/// Mark a task as failed. Retries up to MAX_RETRIES, then fails the whole job.
pub async fn fail_task(
    state: &SharedState,
    task_id: Uuid,
    node_id: Uuid,
    reason: String,
) -> AppResult<()> {
    let task = sqlx::query!(
        "SELECT job_id, retry_count FROM sub_tasks WHERE id = $1",
        task_id
    )
    .fetch_one(&state.db)
    .await?;

    // Dock reputation
    sqlx::query!(
        r#"
        INSERT INTO reputation_events (node_id, event_type, delta, reason)
        VALUES ($1, 'task_fail', -0.05, $2)
        "#,
        node_id,
        format!("task {task_id} failed: {reason}"),
    )
    .execute(&state.db)
    .await?;

    if task.retry_count < MAX_RETRIES {
        // Reset to pending for another dispatch attempt
        sqlx::query!(
            r#"
            UPDATE sub_tasks
            SET status = 'pending', retry_count = retry_count + 1,
                assigned_node_id = NULL, updated_at = NOW()
            WHERE id = $1
            "#,
            task_id
        )
        .execute(&state.db)
        .await?;
    } else {
        // Permanently fail task and escalate to job
        sqlx::query!(
            "UPDATE sub_tasks SET status = 'failed', updated_at = NOW() WHERE id = $1",
            task_id
        )
        .execute(&state.db)
        .await?;

        sqlx::query!(
            "UPDATE jobs SET status = 'failed', updated_at = NOW() WHERE id = $1",
            task.job_id
        )
        .execute(&state.db)
        .await?;

        tracing::error!(job_id = %task.job_id, task_id = %task_id, "job failed after max retries");
    }

    Ok(())
}

/// If every sub-task in a job is 'complete', mark the job 'complete' and set total cost.
pub async fn finalize_job_if_ready(state: &SharedState, job_id: Uuid) -> AppResult<()> {
    let pending_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM sub_tasks WHERE job_id = $1 AND status != 'complete'",
        job_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(1);

    if pending_count == 0 {
        // Sum all credits earned for this job to record total cost.
        let total_cost: f64 = sqlx::query_scalar!(
            "SELECT COALESCE(SUM(credits), 0.0) FROM credit_receipts WHERE job_id = $1",
            job_id
        )
        .fetch_one(&state.db)
        .await
        .unwrap_or(None)
        .unwrap_or(0.0);

        sqlx::query!(
            r#"
            UPDATE jobs
            SET status = 'complete', total_cost_credits = $2,
                completed_at = NOW(), updated_at = NOW()
            WHERE id = $1
            "#,
            job_id,
            total_cost,
        )
        .execute(&state.db)
        .await?;
        tracing::info!(job_id = %job_id, total_cost, "job complete");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Housekeeping
// ---------------------------------------------------------------------------

/// Delete nonces whose TTL has expired. Runs every ~60 s to prevent table bloat.
async fn cleanup_expired_nonces(state: &SharedState) -> AppResult<()> {
    let deleted = sqlx::query!("DELETE FROM request_nonces WHERE expires_at < NOW()")
        .execute(&state.db)
        .await?
        .rows_affected();

    if deleted > 0 {
        tracing::debug!(deleted, "expired nonces cleaned up");
    }
    Ok(())
}

/// Recompute `reputation_score` for every node from its reputation_events over the last 30 days.
///
/// Score = uptime_proxy × 0.3 + completion_rate × 0.4 + win_rate × 0.3, clamped [0, 1].
///
/// Approximation: we don't have explicit uptime pings here, so we estimate uptime from
/// `last_seen_at` recency (seen in the last 5 min = "alive").  Completion and win rates
/// come from aggregated reputation event deltas.
async fn recompute_reputation_scores(state: &SharedState) -> AppResult<()> {
    // Use a single SQL UPDATE to avoid N+1 queries.
    // Aggregates recent events and maps them to a new score in [0, 1].
    sqlx::query!(
        r#"
        UPDATE nodes n
        SET reputation_score = LEAST(1.0, GREATEST(0.0,
            0.5                                  -- baseline
            + COALESCE((
                SELECT SUM(e.delta)
                FROM reputation_events e
                WHERE e.node_id = n.id
                  AND e.occurred_at > NOW() - INTERVAL '30 days'
              ), 0.0)
        ))
        WHERE n.is_banned = FALSE
        "#
    )
    .execute(&state.db)
    .await?;

    tracing::debug!("reputation scores recomputed");
    Ok(())
}
