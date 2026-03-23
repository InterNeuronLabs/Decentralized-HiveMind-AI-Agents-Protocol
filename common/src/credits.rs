// common/src/credits.rs
// Credit formula — pure functions, no I/O, fully unit-testable.

use crate::types::{AgentRole, NodeTier};

/// Tier multiplier for credit earnings.
pub fn tier_multiplier(tier: &NodeTier) -> f64 {
    match tier {
        NodeTier::Nano => 0.1,
        NodeTier::Edge => 1.0,
        NodeTier::Pro => 3.0,
        NodeTier::Cluster => 8.0,
    }
}

/// Per-node credit share.
///
/// C_node = (T_node × W_role × M_tier) / Σ(T_i × W_i × M_i)  × 0.70 × P_total
///
/// Where:
///   T_node   = tokens processed by this node (in + out)
///   W_role   = role weight (Planner/Aggregator = 2.0, etc.)
///   M_tier   = tier multiplier
///   P_total  = total job payout in credits
pub fn node_credit_share(
    tokens_in: u32,
    tokens_out: u32,
    role: &AgentRole,
    tier: &NodeTier,
    all_tasks: &[(u32, u32, AgentRole, NodeTier)], // (tokens_in, tokens_out, role, tier)
    total_payout: f64,
    jobs_completed: u64,
) -> f64 {
    let node_score = node_weighted_score(tokens_in, tokens_out, role, tier);

    let total_score: f64 = all_tasks
        .iter()
        .map(|(tin, tout, r, t)| node_weighted_score(*tin, *tout, r, t))
        .sum();

    if total_score == 0.0 {
        return 0.0;
    }

    let share = (node_score / total_score) * 0.70 * total_payout;

    // New-node anti-Sybil: 50% rate until 100 jobs verified.
    if jobs_completed < 100 {
        share * 0.5
    } else {
        share
    }
}

fn node_weighted_score(tokens_in: u32, tokens_out: u32, role: &AgentRole, tier: &NodeTier) -> f64 {
    let tokens = (tokens_in + tokens_out) as f64;
    tokens * role.weight() * tier_multiplier(tier)
}

/// Split the total payout among protocol actors.
///   70% → executors (via node_credit_share)
///   20% → orchestrator
///   10% → validation pool
pub struct PayoutSplit {
    pub executor_pool: f64,
    pub orchestrator: f64,
    pub validation_pool: f64,
}

pub fn split_payout(total: f64) -> PayoutSplit {
    PayoutSplit {
        executor_pool: total * 0.70,
        orchestrator: total * 0.20,
        validation_pool: total * 0.10,
    }
}

// ---------------------------------------------------------------------------
// Reputation
// ---------------------------------------------------------------------------

/// Rolling 30-day reputation score.
/// uptime_pct ∈ [0,1], completion_rate ∈ [0,1], validation_win_rate ∈ [0,1]
pub fn reputation_score(uptime_pct: f64, completion_rate: f64, validation_win_rate: f64) -> f64 {
    (uptime_pct * 0.3 + completion_rate * 0.4 + validation_win_rate * 0.3).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentRole, NodeTier};

    #[test]
    fn test_new_node_halved_earnings() {
        let all = vec![(100u32, 200u32, AgentRole::Summarizer, NodeTier::Edge)];
        let credits = node_credit_share(
            100,
            200,
            &AgentRole::Summarizer,
            &NodeTier::Edge,
            &all,
            100.0,
            50,
        );
        // New node → 50% of 70% = 35.0
        assert!((credits - 35.0).abs() < 1e-9);
    }

    #[test]
    fn test_veteran_node_full_earnings() {
        let all = vec![(100u32, 200u32, AgentRole::Summarizer, NodeTier::Edge)];
        let credits = node_credit_share(
            100,
            200,
            &AgentRole::Summarizer,
            &NodeTier::Edge,
            &all,
            100.0,
            100,
        );
        // Veteran node → 70% of 100 = 70.0
        assert!((credits - 70.0).abs() < 1e-9);
    }

    #[test]
    fn test_payout_split_adds_up() {
        let split = split_payout(1000.0);
        let total = split.executor_pool + split.orchestrator + split.validation_pool;
        assert!((total - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn test_reputation_clamped() {
        assert_eq!(reputation_score(1.0, 1.0, 1.0), 1.0);
        assert_eq!(reputation_score(0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn test_zero_tokens_gives_zero_credits() {
        let all = vec![(0u32, 0u32, AgentRole::Planner, NodeTier::Pro)];
        let credits =
            node_credit_share(0, 0, &AgentRole::Planner, &NodeTier::Pro, &all, 100.0, 200);
        assert_eq!(credits, 0.0);
    }
}
