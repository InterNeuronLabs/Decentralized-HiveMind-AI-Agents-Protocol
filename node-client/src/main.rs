// node-client/src/main.rs
//
// CLI entry point. Subcommands:
//   setup            — interactive setup wizard (writes .env)
//   register         — register this node with the orchestrator
//   start            — start the worker loop (consume tasks from Redis)
//   wallet show      — print wallet pubkey and config
//   wallet receipts  — list earned credit receipts
//   job submit       — submit a new AI job
//   job status <id>  — poll the status of a job
//   job list         — list all jobs you have submitted
//   job wait <id>    — block until a job finishes, then print its status

#![forbid(unsafe_code)]

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reqwest::Client;
use secrecy::ExposeSecret;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod runner;
mod ui;
mod wallet;

use config::NodeConfig;
use wallet::{load_or_create_key, ReceiptStore};

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "node-client",
    about = "OS-Project — compute cluster node & job CLI",
    version,
    color = clap::ColorChoice::Auto
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive setup wizard — configure your .env and wallet in one step
    Setup,
    /// Register this node as a compute worker with the orchestrator
    Register,
    /// Start consuming and executing sub-tasks from the Redis stream
    Start,
    /// Wallet management (show key, list receipts)
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },
    /// Submit and manage AI jobs
    Job {
        #[command(subcommand)]
        action: JobAction,
    },
}

#[derive(Subcommand)]
enum WalletAction {
    /// Print the node's Ed25519 public key and config summary
    Show,
    /// List all stored credit receipts with totals
    Receipts,
}

#[derive(Subcommand)]
enum JobAction {
    /// Submit a new inference job to the orchestrator
    Submit {
        /// Prompt text. Use '-' or omit to read from stdin.
        #[arg(short, long)]
        prompt: Option<String>,
        /// Read prompt from a file instead of --prompt / stdin
        #[arg(short, long)]
        file: Option<std::path::PathBuf>,
        /// Job tier: standard | paid | premium
        #[arg(short, long, default_value = "standard")]
        tier: String,
        /// Credit budget cap
        #[arg(short, long, default_value = "10.0")]
        budget: f64,
        /// Deadline in seconds from now
        #[arg(short, long, default_value = "3600")]
        deadline: u64,
        /// Model hint (e.g. llama3-8b)
        #[arg(short, long)]
        model: Option<String>,
        /// Block until the job finishes, then print its final status
        #[arg(short, long)]
        wait: bool,
    },
    /// Poll the status of a submitted job
    Status {
        /// Job UUID returned by `job submit`
        job_id: String,
    },
    /// List all jobs you have submitted (scoped to your wallet key)
    List,
    /// Wait (poll) until a job reaches a terminal state, then print it
    Wait {
        /// Job UUID to watch
        job_id: String,
        /// Polling interval in seconds
        #[arg(short, long, default_value = "3")]
        interval: u64,
    },
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env if present (dev convenience)
    let _ = dotenvy::dotenv();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "node_client=warn".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_target(false),
        )
        .init();

    ui::print_banner(env!("CARGO_PKG_VERSION"));

    let cli = Cli::parse();

    // setup wizard doesn't need a loaded config
    if let Commands::Setup = &cli.command {
        return cmd_setup();
    }

    let config = Arc::new(
        NodeConfig::from_env()
            .context("failed to load config — run `node-client setup` to configure")?,
    );
    let signing_key =
        Arc::new(load_or_create_key(&config.wallet_path).context("failed to load wallet key")?);

    match cli.command {
        Commands::Setup => unreachable!(),
        Commands::Register => cmd_register(&config, &signing_key).await?,
        Commands::Start => cmd_start(config.clone(), signing_key.clone()).await?,
        Commands::Wallet { action } => cmd_wallet(action, &config, &signing_key)?,
        Commands::Job { action } => match action {
            JobAction::Submit {
                prompt,
                file,
                tier,
                budget,
                deadline,
                model,
                wait,
            } => {
                cmd_job_submit(
                    &config,
                    &signing_key,
                    prompt,
                    file,
                    &tier,
                    budget,
                    deadline,
                    model,
                    wait,
                )
                .await?;
            }
            JobAction::Status { job_id } => {
                cmd_job_status(&config, &signing_key, &job_id).await?;
            }
            JobAction::List => {
                cmd_job_list(&config, &signing_key).await?;
            }
            JobAction::Wait { job_id, interval } => {
                cmd_job_wait(&config, &signing_key, &job_id, interval).await?;
            }
        },
    }

    Ok(())
}

// ─── Setup wizard ─────────────────────────────────────────────────────────────

fn cmd_setup() -> Result<()> {
    use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
    use std::io::Write;

    ui::section("Setup Wizard");
    ui::hint("Configures your .env for the node-client.");
    ui::gap();

    let theme = ColorfulTheme::default();

    let orchestrator_url: String = Input::with_theme(&theme)
        .with_prompt("Orchestrator URL")
        .default("http://localhost:8080".into())
        .interact_text()?;

    let redis_url: String = Input::with_theme(&theme)
        .with_prompt("Redis URL")
        .default("redis://localhost:6379".into())
        .interact_text()?;

    let tiers = &[
        "nano  — CPU / ≤3B params  (0.1×)",
        "edge  — GPU ≥8 GB / ≤13B   (1.0×)",
        "pro   — GPU ≥24 GB / ≤70B  (3.0×)",
        "cluster — Multi-GPU / 80+ GB (8.0×)",
    ];
    let tier_vals = &["nano", "edge", "pro", "cluster"];
    let tier_idx = Select::with_theme(&theme)
        .with_prompt("Node tier")
        .items(tiers)
        .default(1)
        .interact()?;
    let node_tier = tier_vals[tier_idx];

    let model_id: String = Input::with_theme(&theme)
        .with_prompt("Model ID")
        .default("llama3-8b".into())
        .interact_text()?;

    let llama_url: String = Input::with_theme(&theme)
        .with_prompt("llama-server URL")
        .default("http://127.0.0.1:8081".into())
        .interact_text()?;

    ui::gap();

    let env_path = std::path::Path::new(".env");
    if env_path.exists() {
        let overwrite = Confirm::with_theme(&theme)
            .with_prompt(".env already exists — overwrite?")
            .default(false)
            .interact()?;
        if !overwrite {
            ui::hint("Setup cancelled. Existing .env is unchanged.");
            return Ok(());
        }
    }

    let content = format!(
        "ORCHESTRATOR_URL={}\nREDIS_URL={}\nNODE_TIER={}\nMODEL_ID={}\nLLAMA_SERVER_URL={}\n",
        orchestrator_url, redis_url, node_tier, model_id, llama_url
    );
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(env_path)?
        .write_all(content.as_bytes())?;

    ui::gap();
    ui::success(".env written.");
    ui::gap();
    ui::hint("Next steps:");
    ui::hint("  node-client register   — register this node with the orchestrator");
    ui::hint("  node-client start      — begin accepting tasks and earning credits");
    ui::gap();
    Ok(())
}

// ─── Worker commands ──────────────────────────────────────────────────────────

async fn cmd_register(
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
) -> Result<()> {
    ui::action("Registering node with orchestrator…");
    ui::gap();
    ui::field("Tier", &config.node_tier);
    ui::field("Model", &config.model_id);
    ui::field("Pubkey", &truncate_hex(&signing_key.pubkey_hex(), 16));
    ui::field("URL", &config.orchestrator_url);
    ui::gap();

    let pb = ui::spinner("Connecting…");

    let tier_cap = capitalise(&config.node_tier);
    let body_val = serde_json::json!({
        "pubkey_hex": signing_key.pubkey_hex(),
        "capabilities": {
            "roles": ["Coder", "Summarizer"],
            "models": [config.model_id],
            "vram_mb": null,
            "tier": tier_cap,
        }
    });
    let body = serde_json::to_string(&body_val)?;
    let client = build_client(config)?;
    let (ts, nonce, sig) = sign_request(signing_key, body.as_bytes());

    let resp = client
        .post(format!("{}/nodes/register", config.orchestrator_url))
        .header("Content-Type", "application/json")
        .header("X-Pubkey", signing_key.pubkey_hex())
        .header("X-Timestamp", &ts)
        .header("X-Nonce", &nonce)
        .header("X-Signature", &sig)
        .body(body)
        .send()
        .await
        .context("registration request failed")?;

    pb.finish_and_clear();

    if resp.status().is_success() {
        ui::success("Node registered successfully.");
        ui::gap();
        ui::hint("Run  node-client start  to begin accepting tasks.");
        ui::gap();
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        ui::failure(&format!("Registration failed ({status}): {body}"));
        anyhow::bail!("registration failed");
    }
    Ok(())
}

async fn cmd_start(
    config: Arc<NodeConfig>,
    signing_key: Arc<common::identity::NodeSigningKey>,
) -> Result<()> {
    let redis_url = config
        .redis_url
        .as_ref()
        .context("REDIS_URL must be set to run the worker")?
        .expose_secret()
        .clone();

    ui::section("Worker Node");
    ui::gap();
    ui::field("Pubkey", &truncate_hex(&signing_key.pubkey_hex(), 16));
    ui::field("Tier", &config.node_tier);
    ui::field("Model", &config.model_id);
    ui::field("Stream", "redis › subtasks");
    ui::gap();
    ui::action("Listening for tasks…");
    ui::gap();

    let client = Arc::new(build_client(&config)?);
    runner::run_worker_loop(config, signing_key, &redis_url, client).await
}

fn cmd_wallet(
    action: WalletAction,
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
) -> Result<()> {
    use std::path::PathBuf;

    match action {
        WalletAction::Show => {
            ui::section("Wallet");
            ui::gap();
            ui::field("Public Key", &signing_key.pubkey_hex());
            ui::field("Tier", &config.node_tier);
            ui::field("Model", &config.model_id);
            ui::field("Orch. URL", &config.orchestrator_url);
            ui::field(
                "Solana",
                config
                    .solana_wallet
                    .as_deref()
                    .unwrap_or("— (set SOLANA_WALLET to enable on-chain redemption)"),
            );
            ui::gap();
        }
        WalletAction::Receipts => {
            let wallet_dir = PathBuf::from(&config.wallet_path)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let store = ReceiptStore::new(&wallet_dir)?;
            let receipts = store.list_all()?;
            let total = store.total_credits()?;

            ui::section("Credit Receipts");
            ui::gap();
            ui::field("Count", &receipts.len().to_string());
            ui::field("Total", &format!("{:.4} credits", total));
            ui::gap();

            if receipts.is_empty() {
                ui::hint("No receipts yet — run  node-client start  to earn credits.");
            } else {
                let rows: Vec<Vec<String>> = receipts
                    .iter()
                    .map(|r| {
                        vec![
                            format!("{}…", &r.nonce_hex[..8.min(r.nonce_hex.len())]),
                            r.job_id.to_string(),
                            format!("{:.4}", r.credits),
                            r.issued_at.to_string(),
                        ]
                    })
                    .collect();
                ui::table(
                    &[
                        ("NONCE", 10),
                        ("JOB ID", 36),
                        ("CREDITS", 10),
                        ("ISSUED", 26),
                    ],
                    &rows,
                );
            }
        }
    }
    Ok(())
}

// ─── Job commands ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn cmd_job_submit(
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
    prompt: Option<String>,
    file: Option<std::path::PathBuf>,
    tier: &str,
    budget: f64,
    deadline: u64,
    model: Option<String>,
    wait: bool,
) -> Result<()> {
    let prompt_text = resolve_prompt(prompt, file)?;

    let tier_val = match tier.to_lowercase().as_str() {
        "standard" => "Standard",
        "paid" => "Paid",
        "premium" => "Premium",
        other => anyhow::bail!("unknown tier '{other}': choose standard | paid | premium"),
    };

    ui::action("Submitting job…");
    ui::gap();
    ui::field("Tier", tier);
    ui::field("Budget", &format!("{budget:.1} credits"));
    ui::field("Model", model.as_deref().unwrap_or("any"));
    let preview: String = prompt_text.chars().take(60).collect();
    ui::field("Prompt", &format!("{preview}…"));
    ui::gap();

    let pb = ui::spinner("Sending to orchestrator…");

    let body_val = serde_json::json!({
        "prompt": prompt_text,
        "model_hint": model,
        "budget_cap_credits": budget,
        "tier": tier_val,
        "deadline_secs": deadline,
        "submitter_pubkey_hex": signing_key.pubkey_hex(),
    });
    let body = serde_json::to_string(&body_val)?;
    let client = build_client(config)?;
    let (ts, nonce, sig) = sign_request(signing_key, body.as_bytes());

    let resp = client
        .post(format!("{}/jobs/submit", config.orchestrator_url))
        .header("Content-Type", "application/json")
        .header("X-Pubkey", signing_key.pubkey_hex())
        .header("X-Timestamp", &ts)
        .header("X-Nonce", &nonce)
        .header("X-Signature", &sig)
        .body(body)
        .send()
        .await
        .context("job submit request failed")?;

    pb.finish_and_clear();

    if resp.status().is_success() {
        let val: serde_json::Value = resp.json().await?;
        let job_id = val["job_id"].as_str().unwrap_or("unknown");
        let status = val["status"].as_str().unwrap_or("pending");
        ui::success("Job submitted!");
        ui::gap();
        ui::field("Job ID", job_id);
        ui::field_colored("Status", &ui::status_colored(status));
        ui::gap();
        ui::hint(&format!("Track:  node-client job status {job_id}"));
        ui::gap();
        if wait {
            cmd_job_wait(config, signing_key, job_id, 3).await?;
        }
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        ui::failure(&format!("Job submit failed ({status}): {body}"));
        anyhow::bail!("job submit failed");
    }
    Ok(())
}

async fn cmd_job_status(
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
    job_id: &str,
) -> Result<()> {
    let pb = ui::spinner("Fetching job status…");

    let client = build_client(config)?;
    let (ts, nonce, sig) = sign_request(signing_key, &[]);

    let resp = client
        .get(format!("{}/jobs/{}", config.orchestrator_url, job_id))
        .header("X-Pubkey", signing_key.pubkey_hex())
        .header("X-Timestamp", &ts)
        .header("X-Nonce", &nonce)
        .header("X-Signature", &sig)
        .send()
        .await
        .context("job status request failed")?;

    pb.finish_and_clear();

    if resp.status().is_success() {
        let val: serde_json::Value = resp.json().await?;
        let status = val["status"].as_str().unwrap_or("-");

        ui::section("Job Status");
        ui::gap();
        ui::field("Job ID", val["job_id"].as_str().unwrap_or("-"));
        ui::field_colored("Status", &ui::status_colored(status));
        if let Some(cost) = val["total_cost_credits"].as_f64() {
            ui::field("Cost", &format!("{cost:.4} credits"));
        }
        ui::field("Created", val["created_at"].as_str().unwrap_or("-"));
        if let Some(done) = val["completed_at"].as_str() {
            ui::field("Done", done);
        }
        ui::gap();
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        ui::failure(&format!("Job status failed ({status}): {body}"));
        anyhow::bail!("job status failed");
    }
    Ok(())
}

async fn cmd_job_list(
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
) -> Result<()> {
    let pb = ui::spinner("Fetching jobs…");

    let client = build_client(config)?;
    let (ts, nonce, sig) = sign_request(signing_key, &[]);

    let resp = client
        .get(format!("{}/jobs", config.orchestrator_url))
        .header("X-Pubkey", signing_key.pubkey_hex())
        .header("X-Timestamp", &ts)
        .header("X-Nonce", &nonce)
        .header("X-Signature", &sig)
        .send()
        .await
        .context("job list request failed")?;

    pb.finish_and_clear();

    if resp.status().is_success() {
        let jobs: Vec<serde_json::Value> = resp.json().await?;
        ui::section("Your Jobs");
        ui::gap();
        if jobs.is_empty() {
            ui::hint("No jobs found for your wallet key.");
        } else {
            let rows: Vec<Vec<String>> = jobs
                .iter()
                .map(|j| {
                    vec![
                        j["job_id"].as_str().unwrap_or("-").to_string(),
                        j["status"].as_str().unwrap_or("-").to_string(),
                        j["total_cost_credits"]
                            .as_f64()
                            .map(|c| format!("{c:.4}"))
                            .unwrap_or_else(|| "—".into()),
                        j["created_at"].as_str().unwrap_or("-").to_string(),
                    ]
                })
                .collect();
            ui::table(
                &[
                    ("JOB ID", 36),
                    ("STATUS", 10),
                    ("CREDITS", 10),
                    ("CREATED", 26),
                ],
                &rows,
            );
        }
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        ui::failure(&format!("Job list failed ({status}): {body}"));
        anyhow::bail!("job list failed");
    }
    Ok(())
}

async fn cmd_job_wait(
    config: &NodeConfig,
    signing_key: &common::identity::NodeSigningKey,
    job_id: &str,
    interval_secs: u64,
) -> Result<()> {
    use std::time::{Duration, Instant};

    let short_id = format!("{}…", &job_id[..job_id.len().min(8)]);
    let pb = ui::spinner(&format!("Waiting for job {short_id}…"));
    let start = Instant::now();

    loop {
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;

        let client = build_client(config)?;
        let (ts, nonce, sig) = sign_request(signing_key, &[]);

        let resp = client
            .get(format!("{}/jobs/{}", config.orchestrator_url, job_id))
            .header("X-Pubkey", signing_key.pubkey_hex())
            .header("X-Timestamp", &ts)
            .header("X-Nonce", &nonce)
            .header("X-Signature", &sig)
            .send()
            .await
            .context("poll request failed")?;

        if !resp.status().is_success() {
            pb.finish_and_clear();
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            ui::failure(&format!("Poll failed ({status}): {body}"));
            anyhow::bail!("poll failed");
        }

        let val: serde_json::Value = resp.json().await?;
        let status = val["status"].as_str().unwrap_or("unknown");
        pb.set_message(format!("Waiting for job {short_id}…  [{status}]"));

        if matches!(status, "complete" | "failed" | "expired") {
            pb.finish_and_clear();
            let elapsed = start.elapsed().as_secs();
            ui::success(&format!("Job {short_id} finished in {elapsed}s"));
            ui::gap();
            ui::field_colored("Status", &ui::status_colored(status));
            if let Some(cost) = val["total_cost_credits"].as_f64() {
                ui::field("Cost", &format!("{cost:.4} credits"));
            }
            ui::gap();
            break;
        }
    }
    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn resolve_prompt(prompt: Option<String>, file: Option<std::path::PathBuf>) -> Result<String> {
    let text = if let Some(p) = prompt {
        if p == "-" {
            read_stdin()?
        } else {
            p
        }
    } else if let Some(f) = file {
        std::fs::read_to_string(&f)
            .with_context(|| format!("failed to read prompt file: {}", f.display()))?
    } else {
        ui::hint("Reading prompt from stdin (Ctrl-D to finish):");
        read_stdin()?
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        anyhow::bail!("prompt is empty");
    }
    Ok(text)
}

fn read_stdin() -> Result<String> {
    use std::io::Read;
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .context("failed to read stdin")?;
    Ok(s)
}

fn build_client(_config: &NodeConfig) -> Result<Client> {
    Ok(Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?)
}

fn sign_request(
    signing_key: &common::identity::NodeSigningKey,
    body: &[u8],
) -> (String, String, String) {
    use sha2::{Digest, Sha256};

    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let nonce = hex::encode(rand::random::<[u8; 32]>());

    let mut h = Sha256::new();
    h.update(body);
    h.update(ts.as_bytes());
    h.update(nonce.as_bytes());
    let msg = h.finalize();

    let sig = signing_key.sign_hex(&msg);
    (ts, nonce, sig)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn truncate_hex(hex: &str, chars: usize) -> String {
    if hex.len() <= chars {
        hex.to_string()
    } else {
        format!("{}…", &hex[..chars])
    }
}
