# os-project

A semi-decentralized agentic AI compute cluster. Anyone can connect a machine as a node and earn credits for contributing compute power (GPU/CPU inference). A central lightweight orchestrator handles job routing, credit accounting, and node registry, while all AI inference runs on volunteer nodes.

> For setup and usage instructions, see [INSTRUCTIONS.md](INSTRUCTIONS.md).

---

## Table of Contents

- [os-project](#os-project)
  - [Table of Contents](#table-of-contents)
  - [Architecture Overview](#architecture-overview)
  - [Project Structure](#project-structure)
  - [License](#license)

---

## Architecture Overview

```
 ┌──────────────────────────────────────────────────────┐
 │                     Orchestrator                      │
 │  ┌──────────┐  ┌───────────┐  ┌────────────────────┐ │
 │  │ HTTP API │  │  Task DAG │  │  Task Manager Loop  │ │
 │  │  (Axum)  │  │  + Privacy│  │  (Redis Streams)   │ │
 │  └──────────┘  └───────────┘  └────────────────────┘ │
 │         │              │                │             │
 │     PostgreSQL       Validator        Redis           │
 └─────────────────────────────────────────────────────-┘
           │                                │
     Job submissions              Sub-task dispatch
           │                                │
 ┌─────────▼────────┐            ┌──────────▼──────────┐
 │   User / Agent   │            │    Volunteer Nodes   │
 │  (API consumers) │            │  (node-client CLI)   │
 └──────────────────┘            └─────────────────────┘
```

**How it works:**

1. A user submits a job (prompt + model hint + budget) via the API.
2. The orchestrator breaks the job into sub-tasks using a DAG (Directed Acyclic Graph), with roles like `Planner`, `Coder`, `Critic`, etc.
3. For Paid/Premium jobs, PII (emails, API keys, card numbers, IPs) is tokenized before dispatch.
4. Sub-tasks are queued on a Redis Stream and consumed by volunteer nodes.
5. Nodes run local inference (via llama-server or direct llama.cpp bindings), compute a proof hash, and POST results back.
6. The orchestrator validates results (proof hash, prompt-injection scan, schema check, content sanitization) and issues signed credit receipts.
7. Nodes accumulate credits locally and can redeem them on-chain via Solana SPL token.

---

## Project Structure

```
os-project/
├── Cargo.toml              # Workspace manifest
├── docker-compose.yml      # Postgres + Redis + Orchestrator
├── common/                 # Shared types, crypto, credit logic (no I/O)
│   └── src/
│       ├── types.rs        # NodeTier, AgentRole, JobRequest, TaskDag, etc.
│       ├── identity.rs     # Ed25519 key management
│       ├── credits.rs      # Credit formula, payout split, reputation score
│       └── tls.rs          # mTLS certificate issuance
├── orchestrator/           # Central server (Axum + Postgres + Redis)
│   ├── migrations/
│   │   └── 001_initial.sql # DB schema (auto-applied on startup)
│   └── src/
│       ├── config.rs       # Env-var config loading
│       ├── main.rs         # Server entrypoint
│       ├── state.rs        # AppState (DB pool, Redis, signing keys)
│       ├── dag.rs          # Planner JSON → validated TaskDag
│       ├── privacy.rs      # PII tokenization / detokenization
│       ├── task_manager.rs # Background sub-task dispatcher
│       ├── validator.rs    # 5-step result validation pipeline
│       ├── error.rs        # Error types → HTTP responses
│       ├── middleware/
│       │   ├── auth.rs     # Ed25519 request signing middleware
│       │   └── rate_limit.rs
│       └── routes/
│           ├── nodes.rs    # /nodes/*
│           ├── jobs.rs     # /jobs/*
│           ├── tasks.rs    # /tasks/*
│           └── admin.rs    # /admin/*
└── node-client/            # Volunteer node CLI binary
    └── src/
        ├── config.rs       # Env-var config loading
        ├── main.rs         # CLI entrypoint (clap)
        ├── runner.rs       # Redis consumer + inference worker
        └── wallet.rs       # Ed25519 key + receipt persistence
```

---

## License

Apache-2.0 — see [LICENSE](LICENSE).

Create a `.env` file in the project root (used by Docker Compose) or export these variables before running locally:

| Variable | Required | Description |
|---|---|---|
| `DATABASE_URL` | ✅ | PostgreSQL connection string, e.g. `postgres://postgres:postgres@localhost:5432/orchestrator` |
| `REDIS_URL` | ✅ | Redis URL, e.g. `redis://localhost:6379` |
| `ORCHESTRATOR_SIGNING_KEY` | ✅ | Hex-encoded 64-byte Ed25519 keypair (signing key) |
| `ADMIN_SIGNING_KEY` | ✅ | Hex-encoded 64-byte Ed25519 admin keypair |
| `ORCHESTRATOR_CA_KEY_PATH` | ✅ | Path to cluster CA private key PEM (for mTLS cert issuance) |
| `SOLANA_RPC_URL` | ✅ | Solana RPC endpoint, e.g. `https://api.devnet.solana.com` |
| `CLUSTER_TOKEN_PROGRAM_ID` | ❌ | Anchor program ID (set after first on-chain deploy) |
| `PORT` | ❌ | HTTP listen port (default: `8080`) |
| `APP_ENV` | ❌ | `development` / `staging` / `production` (default: `development`) |

**Generating signing keys:**

```bash
# Generate a random Ed25519 keypair and print as hex (32-byte seed → 64-byte expanded)
# You can use the node-client wallet to generate and view your key:
cargo run -p node-client -- wallet show
```

### Node Client Environment Variables

Create a `.env` file or export before running:

| Variable | Required | Default | Description |
|---|---|---|---|
| `ORCHESTRATOR_URL` | ✅ | — | Base URL of the orchestrator, e.g. `http://localhost:8080` |
| `CA_CERT_PATH` | ✅ | — | Path to cluster CA certificate PEM |
| `NODE_CERT_PATH` | ✅ | — | Path to this node's TLS certificate PEM |
| `NODE_KEY_PATH` | ✅ | — | Path to this node's TLS private key PEM |
| `REDIS_URL` | ✅ | — | Redis URL, e.g. `redis://localhost:6379` (must be same Redis as orchestrator) |
| `WALLET_PATH` | ❌ | `~/.node-client/wallet.key` | Ed25519 key file path |
| `MODEL_ID` | ❌ | `llama3-8b` | Model slug to advertise during registration |
| `MODEL_PATH` | ❌ | `""` | GGUF model file path (only for `llama-direct` feature) |
| `LLAMA_SERVER_URL` | ❌ | `http://127.0.0.1:8080` | Local llama-server address |
| `NODE_TIER` | ❌ | `edge` | Hardware tier: `nano` / `edge` / `pro` / `cluster` |
| `SOLANA_WALLET` | ❌ | — | Solana public key for on-chain credit redemption |

---

## Running with Docker Compose

The easiest way to get the orchestrator running:

```bash
# 1. Clone the repo
git clone https://github.com/SoorajNair-001/os-project
cd os-project

# 2. Create a .env file with required secrets (see configuration above)
cp .env.example .env
# Edit .env and fill in ORCHESTRATOR_SIGNING_KEY, ADMIN_SIGNING_KEY,
# ORCHESTRATOR_CA_KEY_PATH, SOLANA_RPC_URL

# 3. Start all services (Postgres, Redis, Orchestrator)
docker-compose up

# 4. To rebuild after code changes:
docker-compose up --build

# 5. Run in background:
docker-compose up -d
```

Services started:

| Service | Port | Notes |
|---|---|---|
| `postgres` | `5432` | Database: `orchestrator`, user/pass: `postgres`/`postgres` |
| `redis` | `6379` | Persistent volume |
| `orchestrator` | `8080` | Starts after DB and Redis are healthy. DB migrations run automatically. |

---

## Running Locally (Manual Setup)

### 1. Start Dependencies

```bash
# Start PostgreSQL (example with Homebrew on macOS)
brew services start postgresql@15

# Create the database
createdb orchestrator

# Start Redis
brew services start redis
```

### 2. Run the Orchestrator

```bash
# Set environment variables
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/orchestrator"
export REDIS_URL="redis://localhost:6379"
export ORCHESTRATOR_SIGNING_KEY="<64-byte hex signing key>"
export ADMIN_SIGNING_KEY="<64-byte hex admin signing key>"
export ORCHESTRATOR_CA_KEY_PATH="/path/to/ca.key.pem"
export SOLANA_RPC_URL="https://api.devnet.solana.com"

# Run (migrations are applied automatically on startup)
cargo run -p orchestrator

# Or in release mode:
cargo run -p orchestrator --release
```

The orchestrator will:
- Apply any pending SQL migrations
- Start the background task dispatcher loop (500ms interval)
- Listen for HTTP connections on `0.0.0.0:8080`

### 3. Run a Node Client

```bash
# Set environment variables for the node
export ORCHESTRATOR_URL="http://localhost:8080"
export CA_CERT_PATH="/path/to/ca.crt.pem"
export NODE_CERT_PATH="/path/to/node.crt.pem"
export NODE_KEY_PATH="/path/to/node.key.pem"
export REDIS_URL="redis://localhost:6379"
export NODE_TIER="edge"
export MODEL_ID="llama3-8b"
export LLAMA_SERVER_URL="http://127.0.0.1:8080"  # Your local llama-server

# Step 1: Register this node with the orchestrator
cargo run -p node-client -- register

# Step 2: Start processing jobs
cargo run -p node-client -- start
```

> **Note:** The node client connects to the same Redis instance as the orchestrator to consume sub-tasks from the `subtasks` stream.

---

## Building

```bash
# Build all crates
cargo build --workspace

# Release build (LTO enabled, smaller/faster binaries)
cargo build --workspace --release

# Build a specific crate
cargo build -p orchestrator
cargo build -p node-client

# Build node client with direct llama.cpp inference (no external llama-server required)
cargo build -p node-client --features llama-direct
```

Compiled binaries are placed in `target/debug/` or `target/release/`.

---

## API Reference

### Authentication

Every API call (except `GET /health`) requires four HTTP headers for Ed25519 request signing:

| Header | Description |
|---|---|
| `X-Pubkey` | Hex-encoded 32-byte Ed25519 verifying key |
| `X-Timestamp` | Unix time in **milliseconds** (must be within ±30s of server time) |
| `X-Nonce` | Hex-encoded 32-byte random nonce (single-use, 60s TTL) |
| `X-Signature` | Hex Ed25519 signature over `sha256(request_body \|\| timestamp_ms \|\| nonce_hex)` |

Duplicate nonces return `401 Unauthorized`. Stale timestamps return `401 Unauthorized`.

Admin routes additionally require that `X-Pubkey` matches the orchestrator's `ADMIN_SIGNING_KEY` public key.

### Endpoints

#### Public

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Liveness probe — checks DB connectivity. Returns `200 OK` |

#### Nodes

| Method | Path | Description |
|---|---|---|
| `POST` | `/nodes/register` | Register or update a node (upsert by pubkey). Body: `NodeCapabilities` JSON |
| `POST` | `/nodes/heartbeat` | Keep-alive — updates `last_seen_at` |
| `GET` | `/nodes` | List active nodes (seen in last 5 min, not banned), sorted by reputation |

Rate limit: 5 registrations per minute per IP.

#### Jobs

| Method | Path | Description |
|---|---|---|
| `POST` | `/jobs/submit` | Submit a new inference job. Body: `JobRequest` JSON |
| `GET` | `/jobs/:id` | Poll job status and total cost |

Rate limit: 10 job submissions per minute per wallet pubkey.

#### Tasks (Node → Orchestrator)

| Method | Path | Description |
|---|---|---|
| `POST` | `/tasks/:id/result` | Submit completed task result (triggers 5-step validation + credit receipt issuance) |

#### Admin

| Method | Path | Description |
|---|---|---|
| `POST` | `/admin/nodes/ban` | Ban a node by pubkey (with reason, optional expiry timestamp) |
| `POST` | `/admin/nodes/unban` | Unban a node by pubkey |
| `GET` | `/admin/nodes/flagged` | List nodes with reputation score < 0.3 or currently banned |

---

## Node Client CLI

```
node-client <COMMAND>

Commands:
  register          Register this machine as a node with the orchestrator
  start             Begin consuming and processing sub-tasks from the queue
  wallet show       Print this node's Ed25519 public key
  wallet receipts   List all stored credit receipts and total earned credits
```

The wallet key is stored at `~/.node-client/wallet.key` (permissions `0600`). Credit receipts are stored as JSON files in `~/.node-client/receipts/`. The orchestrator's signature on each receipt is verified before writing.

---

## Credit System

Credits are earned per completed sub-task using the following formula:

$$C_{node} = \frac{(T_{in} + T_{out}) \times W_{role} \times M_{tier}}{\sum_i (T_i \times W_i \times M_i)} \times 0.70 \times P_{total}$$

**Payout split per job:**

| Recipient | Share |
|---|---|
| Executor nodes | 70% |
| Orchestrator | 20% |
| Validation pool | 10% |

**Hardware tier multipliers:**

| Tier | Multiplier |
|---|---|
| Nano | 0.1× |
| Edge | 1.0× |
| Pro | 3.0× |
| Cluster | 8.0× |

**Agent role weights:**

| Role | Weight |
|---|---|
| Planner, Aggregator | 2.0× |
| Coder, Researcher | 1.5× |
| Critic | 1.2× |
| ApiRelay | 1.0× |
| Summarizer | 0.8× |

**Anti-Sybil:** Nodes with fewer than 100 completed jobs earn 50% of their calculated rate.

**Redemption:** Each receipt is signed by the orchestrator's Ed25519 key and includes a unique nonce (replay-prevention). Receipts expire after 1 hour and can be redeemed on-chain via the Solana SPL token program (requires `SOLANA_WALLET` to be set on the node client).

---

## Development

### Running Tests

```bash
cargo test --workspace
```

### Linting & Formatting

```bash
cargo fmt
cargo clippy -- -D warnings
```

### Database Migrations

Migrations in `orchestrator/migrations/` are applied automatically when the orchestrator starts via `sqlx::migrate!`. To add a new migration, create a numbered SQL file (e.g. `002_your_change.sql`).

### Dependency Auditing

```bash
# cargo-deny checks licenses, security advisories, and duplicate deps
cargo deny check
```

### Logging

The orchestrator and node client use structured `tracing` logs. Control verbosity via the `RUST_LOG` environment variable:

```bash
RUST_LOG=info cargo run -p orchestrator
RUST_LOG=debug cargo run -p node-client -- start
```

---

## License

Apache-2.0 — see [LICENSE](LICENSE).
