// orchestrator/src/dag.rs
// Task DAG engine: takes a planner's JSON output and builds a TaskDag.
// Provides topological dispatch ordering.
#![allow(dead_code)]

use crate::error::{AppError, AppResult};
use chrono::Utc;
use common::types::{AgentRole, NodeTier, SubTask, SubTaskStatus, TaskDag};
use petgraph::algo::toposort;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The JSON structure a Planner node returns.
#[derive(Debug, Deserialize, Serialize)]
pub struct PlannerOutput {
    pub tasks: Vec<PlannerTask>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlannerTask {
    pub role: AgentRole,
    pub prompt_shard: String,
    pub min_tier: NodeTier,
}

/// Parse planner JSON output into a TaskDag, validating:
/// - No cycles in the dependency graph
/// - Edge indices are within bounds
pub fn build_dag(job_id: Uuid, planner_output: &str) -> AppResult<TaskDag> {
    let plan: PlannerOutput = serde_json::from_str(planner_output)
        .map_err(|e| AppError::BadRequest(format!("invalid planner output: {e}")))?;

    if plan.tasks.is_empty() {
        return Err(AppError::BadRequest("planner returned no tasks".into()));
    }
    if plan.tasks.len() > 64 {
        return Err(AppError::BadRequest("DAG too large (max 64 nodes)".into()));
    }

    // Validate edge indices
    for &(from, to) in &plan.edges {
        if from >= plan.tasks.len() || to >= plan.tasks.len() {
            return Err(AppError::BadRequest(format!(
                "edge ({from},{to}) out of bounds (n={})",
                plan.tasks.len()
            )));
        }
        if from == to {
            return Err(AppError::BadRequest("self-loop in DAG".into()));
        }
    }

    // Build sub-tasks
    let tasks: Vec<SubTask> = plan
        .tasks
        .into_iter()
        .map(|pt| SubTask {
            id: Uuid::new_v4(),
            job_id,
            role: pt.role,
            prompt_shard: pt.prompt_shard,
            min_tier: pt.min_tier,
            assigned_node_id: None,
            status: SubTaskStatus::Pending,
            output: None,
            proof_hash_hex: None,
            tokens_in: None,
            tokens_out: None,
            created_at: Utc::now(),
            completed_at: None,
        })
        .collect();

    let dag = TaskDag::new(tasks, plan.edges.clone());

    // Verify no cycles via petgraph topological sort
    let graph = dag.to_digraph();
    toposort(&graph, None).map_err(|_| AppError::BadRequest("DAG contains a cycle".into()))?;

    Ok(dag)
}

/// Returns sub-task indices that are ready to dispatch right now
/// (all predecessors complete, this task still pending).
pub fn ready_to_dispatch(dag: &TaskDag) -> Vec<usize> {
    dag.ready_indices()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_linear_dag() {
        let json = r#"{
            "tasks": [
                {"role": "Planner",    "prompt_shard": "plan this",  "min_tier": "Nano"},
                {"role": "Summarizer", "prompt_shard": "summarize",   "min_tier": "Nano"}
            ],
            "edges": [[0, 1]]
        }"#;
        let dag = build_dag(Uuid::new_v4(), json).unwrap();
        assert_eq!(dag.tasks.len(), 2);
        let ready = ready_to_dispatch(&dag);
        assert_eq!(ready, vec![0]); // only task 0 is unblocked initially
    }

    #[test]
    fn cyclic_dag_is_rejected() {
        let json = r#"{
            "tasks": [
                {"role": "Planner",    "prompt_shard": "a", "min_tier": "Nano"},
                {"role": "Summarizer", "prompt_shard": "b", "min_tier": "Nano"}
            ],
            "edges": [[0, 1], [1, 0]]
        }"#;
        assert!(build_dag(Uuid::new_v4(), json).is_err());
    }

    #[test]
    fn out_of_bounds_edge_rejected() {
        let json = r#"{
            "tasks": [
                {"role": "Planner", "prompt_shard": "x", "min_tier": "Nano"}
            ],
            "edges": [[0, 5]]
        }"#;
        assert!(build_dag(Uuid::new_v4(), json).is_err());
    }
}
