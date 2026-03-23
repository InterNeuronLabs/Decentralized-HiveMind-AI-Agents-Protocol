// common/src/types.rs
// Core domain types shared between orchestrator and node-client.

use chrono::{DateTime, Utc};
use petgraph::graph::DiGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeTier {
    Nano,    // CPU / 8 GB RAM — models ≤3B params
    Edge,    // GPU ≥8 GB VRAM — models ≤13B params
    Pro,     // GPU ≥24 GB VRAM — models ≤70B params
    Cluster, // Multi-GPU / 80 GB+ — frontier models
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AgentRole {
    Planner,
    Researcher,
    Coder,
    Critic,
    Summarizer,
    Aggregator,
    ApiRelay,
}

impl AgentRole {
    /// Credit weight multiplier for this role.
    pub fn weight(&self) -> f64 {
        match self {
            AgentRole::Planner | AgentRole::Aggregator => 2.0,
            AgentRole::Coder | AgentRole::Researcher => 1.5,
            AgentRole::Critic => 1.2,
            AgentRole::Summarizer => 0.8,
            AgentRole::ApiRelay => 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    pub roles: Vec<AgentRole>,
    pub models: Vec<String>, // GGUF model names available locally
    pub vram_mb: Option<u32>,
    pub tier: NodeTier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: Uuid,
    /// Ed25519 verifying key (hex-encoded public key).
    pub pubkey_hex: String,
    pub capabilities: NodeCapabilities,
    pub reputation_score: f64,
    pub jobs_completed: u64,
    pub is_banned: bool,
    pub registered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Job tier (privacy level)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobTier {
    /// Single executor, 5% random Critic audit. No PII sharding.
    Standard,
    /// Executor + Critic pre-delivery review. PII sharding applied.
    Paid,
    /// Two independent executors + Aggregator reconciles. Full PII sharding.
    Premium,
}

// ---------------------------------------------------------------------------
// Job & Sub-task
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRequest {
    /// Prompt text (max 32 KB enforced at API layer).
    pub prompt: String,
    /// Model preference, e.g. "llama3-8b". Validated against an allow-list.
    pub model_hint: Option<String>,
    pub budget_cap_credits: f64,
    pub tier: JobTier,
    /// Deadline seconds from now.
    pub deadline_secs: u64,
    /// Submitter's Ed25519 public key (hex).
    pub submitter_pubkey_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubTaskStatus {
    Pending,
    Dispatched,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: Uuid,
    pub job_id: Uuid,
    pub role: AgentRole,
    /// Prompt shard delivered to the node. May be PII-tokenized.
    pub prompt_shard: String,
    /// Required model capability minimum.
    pub min_tier: NodeTier,
    /// Node assigned to execute this task.
    pub assigned_node_id: Option<Uuid>,
    pub status: SubTaskStatus,
    pub output: Option<String>,
    /// sha256(prompt_shard_bytes || output_bytes) submitted by the node.
    pub proof_hash_hex: Option<String>,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Task DAG
// ---------------------------------------------------------------------------

/// A node in the DAG represents a sub-task index.
/// Edges represent "must complete before" dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDag {
    /// Ordered list of sub-tasks.
    pub tasks: Vec<SubTask>,
    /// Adjacency: tasks[i] must complete before tasks[j] — stored as (i, j) pairs.
    pub edges: Vec<(usize, usize)>,
}

impl TaskDag {
    pub fn new(tasks: Vec<SubTask>, edges: Vec<(usize, usize)>) -> Self {
        Self { tasks, edges }
    }

    /// Returns indices of tasks whose dependencies are all complete.
    pub fn ready_indices(&self) -> Vec<usize> {
        let n = self.tasks.len();
        let mut blocked = vec![false; n];
        for &(from, to) in &self.edges {
            if self.tasks[from].status != SubTaskStatus::Complete {
                blocked[to] = true;
            }
        }
        (0..n)
            .filter(|&i| !blocked[i] && self.tasks[i].status == SubTaskStatus::Pending)
            .collect()
    }

    /// Build a `petgraph::DiGraph` for topological sort / visualization.
    pub fn to_digraph(&self) -> DiGraph<Uuid, ()> {
        let mut g = DiGraph::new();
        let nodes: Vec<_> = self.tasks.iter().map(|t| g.add_node(t.id)).collect();
        for &(from, to) in &self.edges {
            g.add_edge(nodes[from], nodes[to], ());
        }
        g
    }
}

// ---------------------------------------------------------------------------
// Credit Receipt
// ---------------------------------------------------------------------------

/// Signed by the orchestrator after a sub-task completes successfully.
/// Replay-resistant: nonce + issued_at + expires_at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditReceipt {
    pub id: Uuid,
    pub job_id: Uuid,
    pub sub_task_id: Uuid,
    /// Recipient node's public key (hex).
    pub node_pubkey_hex: String,
    pub credits: f64,
    pub tokens_in: u32,
    pub tokens_out: u32,
    /// Random 32-byte nonce (hex). Prevents replay.
    pub nonce_hex: String,
    pub issued_at: DateTime<Utc>,
    /// Receipts expire after 1 hour; redemption after expiry is rejected.
    pub expires_at: DateTime<Utc>,
    /// Ed25519 signature over canonical JSON of all fields above (hex).
    pub orchestrator_sig_hex: String,
}

impl CreditReceipt {
    /// Returns `true` if the receipt is still within its validity window.
    pub fn is_valid_time(&self) -> bool {
        let now = Utc::now();
        now >= self.issued_at && now <= self.expires_at
    }

    /// Canonical bytes to sign/verify: deterministic JSON (sorted keys).
    pub fn signable_bytes(&self) -> Vec<u8> {
        // Exclude the signature field itself.
        let payload = serde_json::json!({
            "id": self.id,
            "job_id": self.job_id,
            "sub_task_id": self.sub_task_id,
            "node_pubkey_hex": self.node_pubkey_hex,
            "credits": self.credits,
            "tokens_in": self.tokens_in,
            "tokens_out": self.tokens_out,
            "nonce_hex": self.nonce_hex,
            "issued_at": self.issued_at,
            "expires_at": self.expires_at,
        });
        serde_json::to_vec(&payload).expect("CreditReceipt serialization is infallible")
    }
}

// ---------------------------------------------------------------------------
// PII placeholder map (in-memory only, ZeroizeOnDrop)
// ---------------------------------------------------------------------------

/// Maps UUID placeholder → original value. Never written to disk.
/// Manually drops values by zeroing each string.
#[derive(Debug, Default)]
pub struct PiiMap(pub HashMap<String, String>);

impl Drop for PiiMap {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for v in self.0.values_mut() {
            v.zeroize();
        }
        self.0.clear();
    }
}

impl PiiMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, placeholder: String, original: String) {
        self.0.insert(placeholder, original);
    }

    pub fn detokenize(&self, text: &str) -> String {
        let mut result = text.to_owned();
        for (placeholder, original) in &self.0 {
            result = result.replace(placeholder.as_str(), original.as_str());
        }
        result
    }
}
