# Decentralized HiveMind AI Agents Protocol

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-orange.svg)](https://www.rust-lang.org)
[![Build](https://img.shields.io/badge/build-passing-brightgreen.svg)]()

A semi-decentralized protocol for running collaborative multi-agent AI workloads across a permissionless network of volunteer compute nodes. Anyone can contribute a machine — GPU or CPU — and earn credits for inference work. A lightweight central orchestrator handles job routing, task DAG planning, and credit accounting; all AI inference runs on the volunteer nodes.

---

## Table of Contents

- [How It Works](#how-it-works)
- [Architecture](#architecture)
- [Project Structure](#project-structure)
- [Quick Start](#quick-start)
  - [Docker Compose (Recommended)](#docker-compose-recommended)
  - [Manual Local Setup](#manual-local-setup)
- [Running a Node](#running-a-node)
- [Submitting a Job](#submitting-a-job)
- [Configuration Reference](#configuration-reference)
- [API Overview](#api-overview)
- [Credit System](#credit-system)
- [Security Model](#security-model)
- [Contributing](#contributing)
- [License](#license)

---

## How It Works

1. **Submit a job** — A user or autonomous agent sends a prompt + model hint + budget to the orchestrator REST API.
2. **DAG planning** — The orchestrator decomposes the job into sub-tasks using a Directed Acyclic Graph with roles like `Planner`, `Coder`, `Critic`, `Summarizer`, etc.
3. **Privacy filtering** — For paid/premium jobs, PII (emails, API keys, card numbers, IPs) is tokenized before any data leaves the orchestrator.
4. **Dispatch** — Sub-tasks are enqueued on a Redis Stream. Volunteer nodes subscribe and pull work matching their hardware tier.
5. **Inference** — Nodes run local inference via `llama-server` or direct `llama.cpp` bindings, compute a proof hash, and POST results back.
6. **Validation** — The orchestrator runs a 5-step pipeline: proof hash check → prompt-injection scan → schema validation → content sanitization → deduplication.
7. **Credits** — Valid results earn signed credit receipts. Nodes accumulate credits locally and can redeem them on-chain via a Solana SPL token program.

---

## Architecture

```
 ┌─────────────────────────────────────────────────────────┐
 │                       Orchestrator                       │
 │                                                          │
 │   ┌──────────┐   ┌────────────┐   ┌──────────────────┐  │
 │   │ HTTP API │   │  Task DAG  │   │  Task Manager    │  │
 │   │  (Axum)  │   │ + Privacy  │   │  (Redis Streams) │  │
 │   └────┬─────┘   └─────┬──────┘   └────────┬─────────┘  │
 │        │               │                   │             │
 │   PostgreSQL        Validator            Redis           │
 └────────────────────────────────────────────────────────-┘
          │                                   │
    Job submissions                 Sub-task dispatch
          │                                   │
 ┌────────▼──────────┐           ┌────────────▼────────────┐
 │   User / Agent    │           │     Volunteer Nodes      │
 │  (API consumers)  │           │   (node-client CLI)      │
 └───────────────────┘           └─────────────────────────┘
```

### Key Components

| Component | Technology | Role |
|---|---|---|
| Orchestrator | Axum, SQLx, Redis | HTTP API, DAG planner, task dispatcher, credit ledger |
| node-client | Tokio, Ratatui | Volunteer node worker with terminal UI |
| common | Pure Rust | Shared types, Ed25519 identity, credit formula, mTLS |
| Database | PostgreSQL 15 | Job/task/node registry and credit receipts |
| Queue | Redis Streams | Sub-task dispatch and result ingestion |
| On-chain | Solana SPL | Credit redemption as fungible tokens |

---

## Project Structure

```
Decentralized-HiveMind-AI-Agents-Protocol/
├── Cargo.toml                  # Workspace manifest
├── docker-compose.yml          # Postgres + Redis + Orchestrator
├── common/                     # Shared library (no I/O side effects)
│   └── src/
│       ├── types.rs            # NodeTier, AgentRole, JobRequest, TaskDag
│       ├── identity.rs         # Ed25519 key management
│       ├── credits.rs          # Credit formula, payouts, reputation score
│       └── tls.rs              # mTLS certificate issuance
├── orchestrator/               # Central server
│   ├── migrations/
│   │   └── 001_initial.sql     # DB schema (auto-applied on startup)
│   └── src/
│       ├── main.rs             # Server entrypoint
│       ├── config.rs           # Env-var config loading
│       ├── state.rs            # AppState (DB pool, Redis, signing keys)
│       ├── dag.rs              # Planner JSON → validated TaskDag
│       ├── privacy.rs          # PII tokenization / detokenization
│       ├── task_manager.rs     # Background sub-task dispatcher loop
│       ├── validator.rs        # 5-step result validation pipeline
│       ├── error.rs            # Error types → HTTP responses
│       ├── middleware/
│       │   ├── auth.rs         # Ed25519 request signing middleware
│       │   └── rate_limit.rs
│       └── routes/
│           ├── nodes.rs        # /nodes/*
│           ├── jobs.rs         # /jobs/*
│           ├── tasks.rs        # /tasks/*
│           └── admin.rs        # /admin/*
├── node-client/                # Volunteer node CLI binary
│   └── src/
│       ├── main.rs             # CLI entrypoint (clap + interactive setup)
│       ├── config.rs           # Env-var config
│       ├── runner.rs           # Redis consumer + inference worker
│       ├── wallet.rs           # Ed25519 key + credit receipt persistence
│       └── ui.rs               # Ratatui terminal UI
└── keygen/                     # Key generation utility
    └── src/main.rs
```

---

## Quick Start

### Prerequisites

| Tool | Version | Notes |
|---|---|---|
| [Rust](https://rustup.rs) | 1.82+ | Required for local builds |
| [Docker + Compose](https://docs.docker.com/get-docker/) | v2+ | Required for Docker path |
| PostgreSQL | 15+ | Needed for manual setup |
| Redis | 7+ | Needed for manual setup |
| llama-server | any | Required on volunteer nodes |

### Docker Compose (Recommended)

The fastest way to run the orchestrator. No Rust installation needed.

```bash
# 1. Clone
git clone https://github.com/InterNeuronLabs/Decentralized-HiveMind-AI-Agents-Protocol
cd Decentralized-HiveMind-AI-Agents-Protocol

# 2. Generate signing keys
cargo run -p keygen

# 3. Create your .env (see Configuration Reference below)
cp .env.example .env
# Fill in ORCHESTRATOR_SIGNING_KEY, ADMIN_SIGNING_KEY, ORCHESTRATOR_CA_KEY_PATH, SOLANA_RPC_URL

# 4. Start all services
docker compose up

# Rebuild after code changes:
docker compose up --build
```

Services started:

| Service | Port | Notes |
|---|---|---|
| `postgres` | `5432` | DB: `orchestrator`, credentials: `postgres`/`postgres` |
| `redis` | `6379` | Persistent volume |
| `orchestrator` | `8080` | Starts after DB + Redis are healthy; migrations run automatically |

### Manual Local Setup

```bash
# 1. Start PostgreSQL and create the database
brew services start postgresql@15   # macOS example
createdb orchestrator

# 2. Start Redis
brew services start redis

# 3. Export required environment variables
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/orchestrator"
export REDIS_URL="redis://localhost:6379"
export ORCHESTRATOR_SIGNING_KEY="<64-byte hex key>"
export ADMIN_SIGNING_KEY="<64-byte hex key>"
export ORCHESTRATOR_CA_KEY_PATH="/path/to/ca.key.pem"
export SOLANA_RPC_URL="https://api.devnet.solana.com"

# 4. Run (migrations apply automatically)
cargo run -p orchestrator
```

---

## Running a Node

Run the interactive setup wizard for first-time configuration:

```bash
cargo run -p node-client -- setup
```

Or configure via environment variables and start the worker directly:

```bash
export ORCHESTRATOR_URL="http://localhost:8080"
export CA_CERT_PATH="/path/to/ca.crt.pem"
export NODE_CERT_PATH="/path/to/node.crt.pem"
export NODE_KEY_PATH="/path/to/node.key.pem"
export REDIS_URL="redis://localhost:6379"
export NODE_TIER="edge"          # nano | edge | pro | cluster
export MODEL_ID="llama3-8b"
export LLAMA_SERVER_URL="http://127.0.0.1:8080"

cargo run -p node-client -- run
```

View your wallet and earned credits:

```bash
cargo run -p node-client -- wallet show
```

---

## Submitting a Job

```bash
curl -X POST http://localhost:8080/jobs \
  -H "Content-Type: application/json" \
  -H "X-Node-Id: <your-node-id>" \
  -H "X-Signature: <ed25519-signature>" \
  -d '{
    "prompt": "Write a Rust function that parses a JWT without external crates",
    "model_hint": "llama3-8b",
    "budget": 100,
    "tier": "edge"
  }'
```

Poll for results:

```bash
curl http://localhost:8080/jobs/<job-id>
```

---

## Configuration Reference

### Orchestrator Environment Variables

| Variable | Required | Description |
|---|---|---|
| `DATABASE_URL` | ✅ | PostgreSQL connection string |
| `REDIS_URL` | ✅ | Redis URL |
| `ORCHESTRATOR_SIGNING_KEY` | ✅ | Hex-encoded 64-byte Ed25519 keypair |
| `ADMIN_SIGNING_KEY` | ✅ | Hex-encoded 64-byte Ed25519 admin keypair |
| `ORCHESTRATOR_CA_KEY_PATH` | ✅ | Path to cluster CA private key PEM |
| `SOLANA_RPC_URL` | ✅ | Solana RPC endpoint |
| `CLUSTER_TOKEN_PROGRAM_ID` | ❌ | Anchor program ID (set after on-chain deploy) |
| `PORT` | ❌ | HTTP listen port (default: `8080`) |
| `APP_ENV` | ❌ | `development` / `staging` / `production` (default: `development`) |

### Node Client Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `ORCHESTRATOR_URL` | ✅ | — | Orchestrator base URL |
| `CA_CERT_PATH` | ✅ | — | Cluster CA certificate PEM path |
| `NODE_CERT_PATH` | ✅ | — | Node TLS certificate PEM path |
| `NODE_KEY_PATH` | ✅ | — | Node TLS private key PEM path |
| `REDIS_URL` | ✅ | — | Must be the same Redis instance as the orchestrator |
| `WALLET_PATH` | ❌ | `~/.node-client/wallet.key` | Ed25519 key file path |
| `MODEL_ID` | ❌ | `llama3-8b` | Model slug advertised on registration |
| `MODEL_PATH` | ❌ | `""` | GGUF model path (only for `llama-direct` feature) |
| `LLAMA_SERVER_URL` | ❌ | `http://127.0.0.1:8080` | Local llama-server address |
| `NODE_TIER` | ❌ | `edge` | Hardware tier: `nano` / `edge` / `pro` / `cluster` |
| `SOLANA_WALLET` | ❌ | — | Solana public key for credit redemption |

---

## API Overview

| Method | Route | Auth | Description |
|---|---|---|---|
| `POST` | `/jobs` | Signed | Submit a new AI job |
| `GET` | `/jobs/:id` | — | Poll job status and results |
| `GET` | `/nodes` | — | List registered nodes |
| `POST` | `/nodes/register` | Signed | Register a new volunteer node |
| `POST` | `/tasks/:id/result` | Signed | Submit sub-task result (nodes only) |
| `GET` | `/admin/stats` | Admin | Cluster-wide statistics |

Full API reference: [INSTRUCTIONS.md](INSTRUCTIONS.md)

---

## Credit System

Credits are earned per validated sub-task result. The payout formula accounts for:

- **Node tier** — `nano` < `edge` < `pro` < `cluster`
- **Task complexity** — token count, model size, latency
- **Reputation score** — weighted from historical acceptance rate

Earned credits are stored as signed receipts in the node's local wallet and can be redeemed on Solana devnet (mainnet support planned). The on-chain SPL token program ID is configurable via `CLUSTER_TOKEN_PROGRAM_ID`.

---

## Security Model

- **Mutual TLS (mTLS)** — Every node receives a certificate signed by the cluster CA. All node↔orchestrator traffic is authenticated at the transport layer.
- **Ed25519 request signing** — All API requests include a cryptographic signature over the request body + timestamp, preventing replay attacks.
- **PII tokenization** — Before dispatching tasks, the orchestrator scans for and replaces sensitive data (emails, keys, IPs, card numbers) with opaque tokens reversed only on result ingestion.
- **Proof hashing** — Nodes include a hash over `(task_id + prompt_hash + output)` so results can be verified for authenticity.
- **Prompt injection detection** — The validator scans node results for common injection patterns before accepting them.
- **Rate limiting** — Per-IP rate limiting on all public routes via tower middleware.

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. By contributing, you agree to the [Contributor License Agreement](CLA.md).

---

## License

[MIT](LICENSE) — Copyright (c) 2026 InterNeuronLabs
