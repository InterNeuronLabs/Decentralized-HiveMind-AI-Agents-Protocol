# os-project — Step-by-Step Instructions

A semi-decentralized agentic AI compute cluster written in Rust. Volunteer nodes contribute GPU/CPU inference power and earn credits (redeemable as Solana SPL tokens). A central orchestrator handles job routing, credit accounting, and node registry.

The **node-client** binary has an aesthetic terminal UI — colored output, animated spinners, an interactive setup wizard (`node-client setup`), and styled tables throughout.

This guide walks through two paths:
- **Path A — Docker Compose** (recommended, minimal setup)
- **Path B — Local Manual Setup** (for development or nodes without Docker)

---

## Table of Contents

- [Path A: Docker Compose (Orchestrator)](#path-a-docker-compose-orchestrator)
- [Path B: Local Manual Setup (Orchestrator)](#path-b-local-manual-setup-orchestrator)
- [Running a Volunteer Node](#running-a-volunteer-node)
- [Submitting AI Jobs](#submitting-ai-jobs)
- [Managing Your Wallet & Credits](#managing-your-wallet--credits)
- [Reference: All Environment Variables](#reference-all-environment-variables)
- [Reference: API Routes](#reference-api-routes)
- [Reference: Build, Test & Lint Commands](#reference-build-test--lint-commands)
- [Reference: Node Tiers & Credit Formula](#reference-node-tiers--credit-formula)

---

## 1. Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| [Rust](https://rustup.rs) | 1.82+ | Build all crates |
| [Docker + Docker Compose](https://docs.docker.com/get-docker/) | v2+ | Containerized deployment |
| PostgreSQL | 15+ | Orchestrator database |
| Redis | 7+ | Task queue (Redis Streams) |
| llama-server or compatible | — | Local inference on node |

---

## Path A: Docker Compose (Orchestrator)

> Use this path if you want to run the orchestrator with minimal setup. Only Docker is required — no Rust install needed.

### Step 1 — Install Docker

Download and install [Docker Desktop](https://docs.docker.com/get-docker/) (includes Docker Compose v2).

Verify:
```bash
docker --version
docker compose version
```

### Step 2 — Clone the repository

```bash
git clone https://github.com/SoorajNair-001/os-project
cd os-project
```

### Step 3 — Create your `.env` file

```bash
cp .env.example .env
```

Open `.env` and fill in the required values:

```dotenv
DATABASE_URL=postgres://postgres:postgres@localhost:5432/orchestrator
REDIS_URL=redis://localhost:6379
ORCHESTRATOR_SIGNING_KEY=<hex 64-byte Ed25519 keypair>
ADMIN_SIGNING_KEY=<hex 64-byte Ed25519 admin keypair>
ORCHESTRATOR_CA_KEY_PATH=/path/to/ca.key.pem
SOLANA_RPC_URL=https://api.devnet.solana.com
PORT=8080
APP_ENV=development
```

> See [Reference: All Environment Variables](#reference-all-environment-variables) for the full list and defaults.

### Step 4 — Start all services

```bash
docker compose up
```

This starts three containers:

| Container | Image | Port |
|-----------|-------|------|
| `postgres` | `postgres:15-alpine` | `5432` |
| `redis` | `redis:7-alpine` | `6379` |
| `orchestrator` | Built from `orchestrator/Dockerfile` | `8080` |

Database migrations run automatically on first startup.

### Step 5 — Verify the orchestrator is running

```bash
curl http://localhost:8080/health
# Expected: {"status":"ok","db":"connected"}
```

### Step 6 — Manage the stack

```bash
# Run in the background
docker compose up -d

# Rebuild after code changes
docker compose up --build

# Stop (keeps data)
docker compose down

# Stop and delete all data
docker compose down -v
```

---

## Path B: Local Manual Setup (Orchestrator)

> Use this path for development or when running without Docker.

### Step 1 — Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable
```

Verify:
```bash
cargo --version   # requires 1.82+
```

### Step 2 — Install PostgreSQL and Redis (macOS)

```bash
brew install postgresql@15 redis
brew services start postgresql@15
brew services start redis
```

Create the database:
```bash
createdb orchestrator
```

### Step 3 — Set environment variables

Export these in your shell (or add to a `.env` file in the project root):

```bash
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/orchestrator"
export REDIS_URL="redis://localhost:6379"
export ORCHESTRATOR_SIGNING_KEY="<hex 64-byte Ed25519 keypair>"
export ADMIN_SIGNING_KEY="<hex 64-byte Ed25519 admin keypair>"
export ORCHESTRATOR_CA_KEY_PATH="/path/to/ca.key.pem"
export SOLANA_RPC_URL="https://api.devnet.solana.com"
```

> Optional variables (`PORT`, `APP_ENV`, etc.) are listed in [Reference: All Environment Variables](#reference-all-environment-variables).

### Step 4 — Build the orchestrator

```bash
cargo build -p orchestrator --release
```

### Step 5 — Run the orchestrator

```bash
cargo run -p orchestrator --release
```

The server starts on `0.0.0.0:8080`. Migrations run automatically on first boot.

### Step 6 — Verify

```bash
curl http://localhost:8080/health
# Expected: {"db":"ok","status":"ok"}
```

---

## Running a Volunteer Node

> A node earns credits by executing AI inference tasks dispatched by the orchestrator.

### Step 1 — Install Rust (if not already done)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Step 2 — Install llama.cpp and download a model

**Install llama.cpp** (includes `llama-server`):

```bash
brew install llama.cpp
```

Verify:
```bash
llama-server --version
```

**Download a model** (if you don't have one):

```bash
# Install the Hugging Face CLI
pip3 install -U "huggingface_hub[cli]"

# Add the user scripts directory to PATH (use the dynamic form, not a hardcoded version)
export PATH="$(python3 -m site --user-base)/bin:$PATH"

# Persist it so future shells work too
echo 'export PATH="$(python3 -m site --user-base)/bin:$PATH"' >> ~/.zshrc

# Download a quantized Llama 3 8B model (~4.7 GB)
huggingface-cli download bartowski/Meta-Llama-3-8B-Instruct-GGUF \
  Meta-Llama-3-8B-Instruct-Q4_K_M.gguf \
  --local-dir ~/models
```

**Alternative — download directly with curl** (no Python needed):

```bash
mkdir -p ~/models
curl -L -o ~/models/Meta-Llama-3-8B-Instruct-Q4_K_M.gguf \
  "https://huggingface.co/bartowski/Meta-Llama-3-8B-Instruct-GGUF/resolve/main/Meta-Llama-3-8B-Instruct-Q4_K_M.gguf"
```

**Start the inference server:**

```bash
llama-server -m ~/models/Meta-Llama-3-8B-Instruct-Q4_K_M.gguf --port 8081
```

> Use port `8081` to avoid conflict with the orchestrator on `8080`.

### Step 3 — Run the setup wizard

The node-client includes an interactive setup wizard that prompts for all required values and writes your `.env` automatically:

```bash
cargo run -p node-client -- setup
```

The wizard will ask for:

| Prompt | Default |
|--------|---------|
| Orchestrator URL | `http://localhost:8080` |
| Redis URL | `redis://localhost:6379` |
| Node tier | `edge` (select menu) |
| Model ID | `llama3-8b` |
| llama-server URL | `http://127.0.0.1:8081` |

It writes a `.env` in the project root (asks before overwriting an existing one).

> **Manual alternative** — set variables yourself instead of using the wizard:
> ```bash
> export ORCHESTRATOR_URL="http://localhost:8080"
> export REDIS_URL="redis://localhost:6379"
> export LLAMA_SERVER_URL="http://127.0.0.1:8081"
> export MODEL_ID="llama3-8b"
> export NODE_TIER="edge"   # nano | edge | pro | cluster
> ```

### Step 4 — Generate and view your wallet key

The node-client auto-creates an Ed25519 keypair at `~/.node-client/wallet.key` on first run.

```bash
cargo run -p node-client -- wallet show
```

Shows your public key, tier, model, orchestrator URL, and Solana wallet (if set) in a styled panel.

### Step 5 — Register your node

```bash
cargo run -p node-client -- register
```

Displays a confirmation panel (tier, model, pubkey, URL), then spins while sending your node capabilities to the orchestrator. Prints a green `✓` on success.

### Step 6 — Start the worker loop

```bash
cargo run -p node-client -- start
```

Prints a node info panel, then streams timestamped task events:

```
  [18:04:12]  ⚡  Coder        task a3f92c…
  [18:04:15]  ✓  result submitted  task a3f92c…
```

The node:
1. Subscribes to the Redis Streams `subtasks` queue
2. Executes inference tasks via your llama-server
3. Computes a proof hash: `sha256(prompt_shard || output)`
4. POSTs the result back to `POST /tasks/:id/result`
5. On validation pass, receives a signed `CreditReceipt` saved to `~/.node-client/receipts/`

---

## Submitting AI Jobs

> Jobs can be submitted from any machine with `ORCHESTRATOR_URL` set. If you haven't run the setup wizard yet, either run `node-client setup` or export `ORCHESTRATOR_URL` manually:
> ```bash
> export ORCHESTRATOR_URL="http://localhost:8080"
> ```

### Step 1 — Submit a job

```bash
# Basic prompt
cargo run -p node-client -- job submit --prompt "Summarize the Rust ownership model"

# With tier and budget
cargo run -p node-client -- job submit \
  --prompt "Explain async/await in Rust" \
  --tier paid \
  --budget 25.0

# From a file
cargo run -p node-client -- job submit --file ./my-prompt.txt --tier premium

# From stdin
echo "Write a Rust TCP server" | cargo run -p node-client -- job submit --prompt -

# Block until the job finishes (polls with a live spinner)
cargo run -p node-client -- job submit --prompt "Count to 10" --wait
```

On success the CLI prints a styled confirmation:
```
  ✓ Job submitted!

  Job ID     ›  3e2a1f08-…
  Status     ›  ○ pending

  Track:  node-client job status 3e2a1f08-…
```

**Available flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--prompt <text>` | — | Prompt string, or `-` for stdin |
| `--file <path>` | — | Read prompt from a file |
| `--tier` | `standard` | `standard` \| `paid` \| `premium` |
| `--budget <credits>` | `10.0` | Max credits to spend |
| `--deadline <secs>` | `3600` | Job deadline in seconds |
| `--model <hint>` | — | Preferred model slug |
| `--wait` | `false` | Block until job reaches terminal state |

**Job tiers:**

| Tier | What happens |
|------|-------------|
| `standard` | Single executor node; 5% chance of random Critic audit |
| `paid` | Executor + Critic pre-delivery review; PII is sharded |
| `premium` | Two independent executors + Aggregator reconciliation; full PII sharding |

### Step 2 — Check job status

```bash
cargo run -p node-client -- job status <job-id>
```

Prints a status panel with color-coded status (`✓ complete` / `⟳ running` / `✗ failed`).

### Step 3 — List all your jobs

```bash
cargo run -p node-client -- job list
```

Prints a table with job ID, color-coded status, credits spent, and creation time.

### Step 4 — Wait for a running job

```bash
cargo run -p node-client -- job wait <job-id>

# Custom polling interval
cargo run -p node-client -- job wait <job-id> --interval 5
```

Shows a live spinner updating with the current status until the job reaches a terminal state.

---

## Managing Your Wallet & Credits

### Step 1 — View your public key and config

```bash
cargo run -p node-client -- wallet show
```

Displays a styled panel with your Ed25519 public key, tier, model ID, orchestrator URL, and Solana wallet address (if configured).

### Step 2 — View earned credits

```bash
cargo run -p node-client -- wallet receipts
```

Shows receipt count and total credits, then a table of all `CreditReceipt` files stored in `~/.node-client/receipts/`.

Receipts are:
- Signed by the orchestrator's Ed25519 key
- Replay-resistant (include a nonce + expiry)
- Redeemable on-chain via Solana SPL once `CLUSTER_TOKEN_PROGRAM_ID` is set

---

## Reference: All Environment Variables

### Orchestrator

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | ✅ | — | PostgreSQL connection string |
| `REDIS_URL` | ✅ | — | Redis connection string |
| `ORCHESTRATOR_SIGNING_KEY` | ✅ | — | Hex 64-byte Ed25519 keypair |
| `ADMIN_SIGNING_KEY` | ✅ | — | Hex 64-byte Ed25519 admin keypair |
| `ORCHESTRATOR_CA_KEY_PATH` | ✅ | — | Path to CA private key PEM |
| `SOLANA_RPC_URL` | ✅ | — | Solana RPC URL |
| `CLUSTER_TOKEN_PROGRAM_ID` | ❌ | `None` | Anchor program ID (after on-chain deploy) |
| `PORT` | ❌ | `8080` | HTTP listen port |
| `APP_ENV` | ❌ | `development` | `development` \| `staging` \| `production` |

### Node Client

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `ORCHESTRATOR_URL` | ✅ | — | Orchestrator base URL |
| `REDIS_URL` | ✅ (for `start`) | — | Redis connection string |
| `WALLET_PATH` | ❌ | `~/.node-client/wallet.key` | Ed25519 wallet key path |
| `CA_CERT_PATH` | ❌ | `None` | mTLS CA cert PEM path |
| `NODE_CERT_PATH` | ❌ | `None` | mTLS node cert PEM path |
| `NODE_KEY_PATH` | ❌ | `None` | mTLS node key PEM path |
| `MODEL_ID` | ❌ | `llama3-8b` | Model slug |
| `MODEL_PATH` | ❌ | `""` | GGUF model path (llama-direct feature) |
| `LLAMA_SERVER_URL` | ❌ | `http://127.0.0.1:8080` | Local inference server URL |
| `NODE_TIER` | ❌ | `edge` | `nano` \| `edge` \| `pro` \| `cluster` |
| `SOLANA_WALLET` | ❌ | `None` | Solana public key for on-chain redemption |

---

## Reference: API Routes

All routes except `GET /health` require four signed headers:

| Header | Value |
|--------|-------|
| `X-Pubkey` | Hex 32-byte Ed25519 verifying key |
| `X-Timestamp` | Unix time in milliseconds (±30s tolerance) |
| `X-Nonce` | Hex 32-byte random nonce (single-use, 60s TTL) |
| `X-Signature` | Ed25519 sig over `sha256(body ∥ timestamp_ms ∥ nonce_hex)` |

The node-client attaches these automatically. For direct API calls you must compute them yourself.

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | None | Health check |
| `POST` | `/nodes/register` | Signed | Register/update node capabilities |
| `POST` | `/nodes/heartbeat` | Signed | Update `last_seen_at` |
| `GET` | `/nodes` | Signed | List registered nodes |
| `POST` | `/jobs/submit` | Signed | Submit a new AI job |
| `GET` | `/jobs` | Signed | List jobs from this wallet |
| `GET` | `/jobs/:id` | Signed | Get job status/result |
| `POST` | `/tasks/:id/result` | Signed | Submit sub-task inference result |
| `POST` | `/admin/nodes/ban` | Admin | Ban a node |
| `POST` | `/admin/nodes/unban` | Admin | Unban a node |
| `GET` | `/admin/nodes/flagged` | Admin | List flagged/banned nodes |

---

## Reference: node-client Commands

| Command | Description |
|---------|-------------|
| `node-client setup` | Interactive setup wizard — configures `.env` with prompts |
| `node-client register` | Register / update this node with the orchestrator |
| `node-client start` | Start the worker loop (earn credits) |
| `node-client wallet show` | Print pubkey, tier, model, orchestrator URL |
| `node-client wallet receipts` | List all earned `CreditReceipt` files with totals |
| `node-client job submit` | Submit a new inference job |
| `node-client job status <id>` | Print job status panel |
| `node-client job list` | List all jobs for your wallet key |
| `node-client job wait <id>` | Poll with live spinner until job finishes |

---

## Reference: Build, Test & Lint Commands

### Build

```bash
# All crates (debug)
cargo build --workspace

# All crates (release)
cargo build --workspace --release

# Orchestrator only
cargo build -p orchestrator --release

# Node-client only
cargo build -p node-client --release

# Node-client with direct llama.cpp (no external server required)
cargo build -p node-client --features llama-direct --release
```

Binaries are written to `target/release/`.

### Test

```bash
cargo test --workspace
```

### Lint & Format

```bash
cargo clippy --workspace -- -D warnings   # lint
cargo fmt --all                           # format
cargo deny check                          # dependency audit
```

### Logging

Controlled by `RUST_LOG`:

```bash
# Default
cargo run -p orchestrator

# Verbose
RUST_LOG=orchestrator=debug,tower_http=debug cargo run -p orchestrator

# Node debug
RUST_LOG=node_client=debug cargo run -p node-client -- start
```

---

## Reference: Node Tiers & Credit Formula

### Tiers

| Tier | Hardware requirement | Max model size | Multiplier |
|------|---------------------|---------------|------------|
| `nano` | CPU / 8 GB RAM | ≤3B params | 0.1× |
| `edge` | GPU ≥8 GB VRAM | ≤13B params | 1.0× |
| `pro` | GPU ≥24 GB VRAM | ≤70B params | 3.0× |
| `cluster` | Multi-GPU / 80 GB+ | Frontier models | 8.0× |

### Payout split per job

| Recipient | Share |
|-----------|-------|
| Executor nodes | 70% |
| Orchestrator | 20% |
| Validation pool | 10% |

### Credit formula

$$C_{node} = \frac{(T_{in} + T_{out}) \times W_{role} \times M_{tier}}{\sum_i (T_i \times W_i \times M_i)} \times 0.70 \times P_{total}$$

- $T_{in}, T_{out}$ — input and output tokens processed
- $W_{role}$ — Planner/Aggregator=2.0, Coder/Researcher=1.5, Critic=1.2, ApiRelay=1.0, Summarizer=0.8
- $M_{tier}$ — tier multiplier from table above
- $P_{total}$ — total credits allocated to the job

### Anti-Sybil

Nodes with fewer than **100 completed jobs** earn **50%** of their calculated rate.

### Reputation score

$$R = 0.3 \times \text{uptime} + 0.4 \times \text{completion\_rate} + 0.3 \times \text{validation\_win\_rate}$$

Clamped to $[0, 1]$. Higher reputation increases task assignment priority.
