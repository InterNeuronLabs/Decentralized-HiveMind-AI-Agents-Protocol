// orchestrator/src/state.rs
// Shared application state threaded through all Axum handlers via Arc.

use crate::config::AppConfig;
use common::identity::NodeSigningKey;
use redis::aio::MultiplexedConnection;
use secrecy::ExposeSecret;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: PgPool,
    pub redis: Arc<Mutex<MultiplexedConnection>>,
    #[allow(dead_code)]
    pub signing_key: Arc<NodeSigningKey>,
    pub admin_signing_key: Arc<NodeSigningKey>,
    pub config: Arc<AppConfig>,
}

impl AppState {
    pub async fn new(config: AppConfig) -> anyhow::Result<Self> {
        // Database pool
        let db = PgPool::connect(config.database_url.expose_secret()).await?;
        sqlx::migrate!("./migrations").run(&db).await?;
        tracing::info!("database migrations applied");

        // Redis connection
        let redis_client = redis::Client::open(config.redis_url.expose_secret().as_str())?;
        let redis_conn = redis_client.get_multiplexed_async_connection().await?;

        // Signing keys
        let signing_key =
            NodeSigningKey::from_hex(config.orchestrator_signing_key.expose_secret())?;
        let admin_signing_key = NodeSigningKey::from_hex(config.admin_signing_key.expose_secret())?;

        tracing::info!(pubkey = %signing_key.pubkey_hex(), "orchestrator identity loaded");

        Ok(Self {
            db,
            redis: Arc::new(Mutex::new(redis_conn)),
            signing_key: Arc::new(signing_key),
            admin_signing_key: Arc::new(admin_signing_key),
            config: Arc::new(config),
        })
    }
}

pub type SharedState = Arc<AppState>;
