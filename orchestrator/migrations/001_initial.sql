-- orchestrator/migrations/001_initial.sql
-- Initial schema for the cluster orchestrator.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ---------------------------------------------------------------------------
-- Nodes
-- ---------------------------------------------------------------------------

CREATE TABLE nodes (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pubkey_hex      TEXT NOT NULL UNIQUE,          -- Ed25519 verifying key (hex)
    capabilities    JSONB NOT NULL,                -- NodeCapabilities
    tier            TEXT NOT NULL,                 -- Nano | Edge | Pro | Cluster
    reputation_score DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    jobs_completed  BIGINT NOT NULL DEFAULT 0,
    is_banned       BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason      TEXT,
    ban_expires_at  TIMESTAMPTZ,
    registered_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX nodes_pubkey ON nodes(pubkey_hex);
CREATE INDEX nodes_tier   ON nodes(tier);
CREATE INDEX nodes_banned ON nodes(is_banned);

-- ---------------------------------------------------------------------------
-- Jobs
-- ---------------------------------------------------------------------------

CREATE TABLE jobs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    submitter_pubkey_hex TEXT NOT NULL,
    -- Prompt stored encrypted with pgcrypto (key = ORCHESTRATOR_DB_ENCRYPT_KEY env var)
    prompt_encrypted    BYTEA NOT NULL,
    model_hint          TEXT,
    budget_cap_credits  DOUBLE PRECISION NOT NULL,
    tier                TEXT NOT NULL,
    deadline_at         TIMESTAMPTZ NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',  -- pending|running|complete|failed
    total_cost_credits  DOUBLE PRECISION,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ
);

CREATE INDEX jobs_status      ON jobs(status);
CREATE INDEX jobs_submitter   ON jobs(submitter_pubkey_hex);

-- ---------------------------------------------------------------------------
-- Sub-tasks
-- ---------------------------------------------------------------------------

CREATE TABLE sub_tasks (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id              UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    role                TEXT NOT NULL,
    prompt_shard        TEXT NOT NULL,             -- may be PII-tokenized
    min_tier            TEXT NOT NULL,
    position            INTEGER NOT NULL DEFAULT 0,
    agent_index         INTEGER NOT NULL DEFAULT 0,
    assigned_node_id    UUID REFERENCES nodes(id),
    status              TEXT NOT NULL DEFAULT 'pending',
    result              TEXT,
    proof_hash_hex      TEXT,
    tokens_in           INTEGER,
    tokens_out          INTEGER,
    retry_count         INTEGER NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ
);

CREATE INDEX sub_tasks_job    ON sub_tasks(job_id);
CREATE INDEX sub_tasks_status ON sub_tasks(status);
CREATE INDEX sub_tasks_node   ON sub_tasks(assigned_node_id);

-- ---------------------------------------------------------------------------
-- Credit receipts
-- ---------------------------------------------------------------------------

CREATE TABLE credit_receipts (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id               UUID NOT NULL REFERENCES jobs(id),
    sub_task_id          UUID NOT NULL REFERENCES sub_tasks(id),
    node_pubkey_hex      TEXT NOT NULL,
    credits              DOUBLE PRECISION NOT NULL,
    tokens_in            INTEGER NOT NULL,
    tokens_out           INTEGER NOT NULL,
    nonce_hex            TEXT NOT NULL UNIQUE,     -- enforces no replay
    issued_at            TIMESTAMPTZ NOT NULL,
    expires_at           TIMESTAMPTZ NOT NULL,
    orchestrator_sig_hex TEXT NOT NULL,
    redeemed_at          TIMESTAMPTZ                -- NULL = not yet redeemed
);

CREATE INDEX receipts_node   ON credit_receipts(node_pubkey_hex);
CREATE INDEX receipts_nonce  ON credit_receipts(nonce_hex);

-- ---------------------------------------------------------------------------
-- Reputation events (append-only — never UPDATE, only INSERT)
-- ---------------------------------------------------------------------------

CREATE TABLE reputation_events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id     UUID NOT NULL REFERENCES nodes(id),
    event_type  TEXT NOT NULL,   -- 'job_complete'|'validation_win'|'validation_loss'|'tamper'|'ban'
    delta       DOUBLE PRECISION NOT NULL,
    reason      TEXT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX rep_events_node ON reputation_events(node_id);

-- ---------------------------------------------------------------------------
-- Nonce store (replay prevention for signed API requests)
-- ---------------------------------------------------------------------------

CREATE TABLE request_nonces (
    nonce_hex   TEXT PRIMARY KEY,
    used_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL
);

-- Nonces expire after 60 seconds; a background job prunes old rows.
CREATE INDEX request_nonces_expires ON request_nonces(expires_at);
