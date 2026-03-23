// node-client/src/runner.rs
//
// Redis XREADGROUP consumer that picks up sub-tasks, runs inference via
// llama-server (HTTP subprocess) or llama-cpp-rs (direct, feature-gated),
// computes the proof hash, and reports the result back to the orchestrator.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use common::identity::NodeSigningKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::sleep;

use crate::config::NodeConfig;

const CONSUMER_GROUP: &str = "node-workers";
const STREAM_KEY: &str = "subtasks";
const BLOCK_MS: u64 = 2_000;
const MAX_IDLE_MS: u64 = 30_000;

// ---------------------------------------------------------------------------
// Task payload received from Redis stream
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TaskPayload {
    task_id: String,
    job_id: String,
    role: String,
    prompt_shard: String,
    min_tier: String,
    agent_index: u32,
}

// ---------------------------------------------------------------------------
// Orchestrator result reporting
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct TaskResult {
    task_id: String,
    output: String,
    proof_hash: String,
    node_pubkey: String,
}

// ---------------------------------------------------------------------------
// Runner loop
// ---------------------------------------------------------------------------

pub async fn run_worker_loop(
    config: Arc<NodeConfig>,
    signing_key: Arc<NodeSigningKey>,
    redis_url: &str,
    client: Arc<Client>,
) -> Result<()> {
    let redis = redis::Client::open(redis_url).context("failed to connect to Redis")?;
    let mut con = redis
        .get_multiplexed_async_connection()
        .await
        .context("Redis connection failed")?;

    // Ensure consumer group exists (ignore BUSYGROUP error if already created)
    let _: redis::RedisResult<()> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(STREAM_KEY)
        .arg(CONSUMER_GROUP)
        .arg("$")
        .arg("MKSTREAM")
        .query_async(&mut con)
        .await;

    let consumer_name = signing_key.pubkey_hex();

    // Also claim stale messages that have been idle > MAX_IDLE_MS
    loop {
        // First try to claim stale messages from other (dead) consumers
        let pending: Vec<redis::Value> = redis::cmd("XAUTOCLAIM")
            .arg(STREAM_KEY)
            .arg(CONSUMER_GROUP)
            .arg(&consumer_name)
            .arg(MAX_IDLE_MS)
            .arg("0-0")
            .arg("COUNT")
            .arg(5)
            .query_async(&mut con)
            .await
            .unwrap_or_default();

        let claimed = extract_stream_entries(&pending);

        // Then read new messages
        let fresh: Vec<redis::Value> = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(CONSUMER_GROUP)
            .arg(&consumer_name)
            .arg("COUNT")
            .arg(5)
            .arg("BLOCK")
            .arg(BLOCK_MS)
            .arg("STREAMS")
            .arg(STREAM_KEY)
            .arg(">")
            .query_async(&mut con)
            .await
            .unwrap_or_default();

        let fresh_entries = extract_stream_entries(&fresh);

        for (entry_id, payload_json) in claimed.into_iter().chain(fresh_entries) {
            match process_task(&payload_json, &config, &signing_key, &client).await {
                Ok(_) => {
                    // ACK the message
                    let _: redis::RedisResult<()> = redis::cmd("XACK")
                        .arg(STREAM_KEY)
                        .arg(CONSUMER_GROUP)
                        .arg(&entry_id)
                        .query_async(&mut con)
                        .await;
                }
                Err(e) => {
                    tracing::error!(entry_id = %entry_id, error = %e, "task processing failed");
                    crate::ui::task_err(&entry_id, &e.to_string());
                    // Don't ACK — message stays for retry / redelivery
                }
            }
        }

        // Brief sleep to avoid hot-looping if stream is empty
        sleep(Duration::from_millis(100)).await;
    }
}

/// Process a single task payload end-to-end.
async fn process_task(
    payload_json: &str,
    config: &NodeConfig,
    signing_key: &NodeSigningKey,
    client: &Client,
) -> Result<()> {
    let task: TaskPayload =
        serde_json::from_str(payload_json).context("failed to parse task payload")?;

    tracing::info!(task_id = %task.task_id, role = %task.role, "processing task");
    crate::ui::task_start(&task.task_id, &task.role);

    // Run inference
    let output = run_inference(&task.prompt_shard, &task.role, config, client).await?;

    // Compute proof hash: sha256(prompt_shard || output)
    let mut hasher = Sha256::new();
    hasher.update(task.prompt_shard.as_bytes());
    hasher.update(output.as_bytes());
    let proof_hash = hex::encode(hasher.finalize());

    // Report result to orchestrator
    let result = TaskResult {
        task_id: task.task_id.clone(),
        output,
        proof_hash,
        node_pubkey: signing_key.pubkey_hex(),
    };

    let body = serde_json::to_string(&result)?;
    let timestamp_ms = chrono::Utc::now().timestamp_millis().to_string();
    let nonce = hex::encode(rand::random::<[u8; 32]>());

    // Sign sha256(body || timestamp_ms || nonce) — must match orchestrator's verification
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher.update(timestamp_ms.as_bytes());
    hasher.update(nonce.as_bytes());
    let msg_hash = hasher.finalize();
    let signature = signing_key.sign_hex(&msg_hash);

    client
        .post(format!(
            "{}/tasks/{}/result",
            config.orchestrator_url, task.task_id
        ))
        .header("Content-Type", "application/json")
        .header("X-Pubkey", signing_key.pubkey_hex())
        .header("X-Timestamp", &timestamp_ms)
        .header("X-Nonce", &nonce)
        .header("X-Signature", &signature)
        .body(body)
        .send()
        .await
        .context("failed to POST task result")?
        .error_for_status()
        .context("orchestrator rejected task result")?;

    crate::ui::task_done(&task.task_id);
    tracing::info!(task_id = %task.task_id, "task result submitted");
    Ok(())
}

/// Run inference using llama-server (subprocess HTTP) or llama-cpp-rs (direct).
async fn run_inference(
    prompt: &str,
    _role: &str,
    config: &NodeConfig,
    client: &Client,
) -> Result<String> {
    #[cfg(feature = "llama-direct")]
    {
        run_inference_direct(prompt, config)
    }

    #[cfg(not(feature = "llama-direct"))]
    {
        run_inference_server(prompt, config, client).await
    }
}

/// Call a local llama-server (OpenAI-compatible /v1/completions).
#[allow(dead_code)]
async fn run_inference_server(
    prompt: &str,
    config: &NodeConfig,
    client: &Client,
) -> Result<String> {
    #[derive(Serialize)]
    struct CompletionRequest<'a> {
        prompt: &'a str,
        max_tokens: u32,
        temperature: f32,
    }

    #[derive(Deserialize)]
    struct CompletionResponse {
        choices: Vec<Choice>,
    }

    #[derive(Deserialize)]
    struct Choice {
        text: String,
    }

    let req = CompletionRequest {
        prompt,
        max_tokens: 512,
        temperature: 0.2,
    };

    let resp: CompletionResponse = client
        .post(format!("{}/v1/completions", config.llama_server_url))
        .json(&req)
        .send()
        .await
        .context("llama-server request failed")?
        .error_for_status()
        .context("llama-server returned error status")?
        .json()
        .await
        .context("failed to parse llama-server response")?;

    resp.choices
        .into_iter()
        .next()
        .map(|c| c.text)
        .context("llama-server returned empty choices")
}

// Stub for direct llama-cpp-rs integration (requires 'llama-direct' feature)
#[cfg(feature = "llama-direct")]
fn run_inference_direct(prompt: &str, config: &NodeConfig) -> Result<String> {
    // TODO: initialise llama_cpp::LlamaModel once at startup and reuse session
    anyhow::bail!("llama-direct feature not yet implemented; use llama-server mode")
}

// ---------------------------------------------------------------------------
// Stream parsing helpers
// ---------------------------------------------------------------------------

/// Extract (entry_id, payload_json) pairs from XREADGROUP / XAUTOCLAIM responses.
/// Redis stream format is deeply nested; we flatten it here.
fn extract_stream_entries(values: &[redis::Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();

    fn walk(v: &redis::Value, out: &mut Vec<(String, String)>) {
        if let redis::Value::Array(items) = v {
            // Try to interpret as (id, [field, value, ...]) pair
            if items.len() == 2 {
                if let (redis::Value::BulkString(id_bytes), redis::Value::Array(fields)) =
                    (&items[0], &items[1])
                {
                    let id = String::from_utf8_lossy(id_bytes).to_string();
                    // fields = ["payload", "<json>"]
                    if fields.len() >= 2 {
                        if let (
                            redis::Value::BulkString(_key),
                            redis::Value::BulkString(val_bytes),
                        ) = (&fields[0], &fields[1])
                        {
                            let payload = String::from_utf8_lossy(val_bytes).to_string();
                            out.push((id, payload));
                            return;
                        }
                    }
                }
            }
            for item in items {
                walk(item, out);
            }
        }
    }

    for v in values {
        walk(v, &mut out);
    }
    out
}
