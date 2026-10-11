//! Shared application state.

use crate::ai::{AgentsAi, AiServices};
use crate::tasks::{SqsTaskQueue, TaskQueue};
use kabados_authz::{Authorizer, LocalCedar, VerifiedPermissions};
use kabados_db::repo::Db;
use kabados_domain::taxonomy::Taxonomy;
use kabados_integrations::Integrations;
use kabados_runtime::Config;
use kabados_runtime::crypto::{Hasher, KmsSealer, LocalSealer, Sealer};
use kabados_runtime::secrets::Secrets;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    pub authz: Arc<dyn Authorizer>,
    pub ix: Integrations,
    pub sealer: Arc<dyn Sealer>,
    pub hasher: Hasher,
    pub tasks: Arc<dyn TaskQueue>,
    pub secrets: Secrets,
    pub ai: Arc<dyn AiServices>,
    taxonomy: RwLock<Option<(Instant, Arc<Taxonomy>)>>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cfg: Config,
        db: Db,
        authz: Arc<dyn Authorizer>,
        ix: Integrations,
        sealer: Arc<dyn Sealer>,
        hasher: Hasher,
        tasks: Arc<dyn TaskQueue>,
        secrets: Secrets,
    ) -> Self {
        Self(Arc::new(Inner {
            cfg,
            db,
            authz,
            ix,
            sealer,
            hasher,
            tasks,
            secrets,
            ai: Arc::new(AgentsAi::new(kabados_agents::mock_runtime())),
            taxonomy: RwLock::new(None),
        }))
    }

    /// Everything in-process: memory store, local Cedar, local adapters.
    pub fn local(cfg: Config, db: Db, ix: Integrations, tasks: Arc<dyn TaskQueue>) -> anyhow::Result<Self> {
        let secrets = Secrets::new(None, cfg.env.clone());
        Ok(Self::new(
            cfg,
            db,
            Arc::new(LocalCedar::new()?),
            ix,
            Arc::new(LocalSealer::new("kabados-local")),
            Hasher::new("kabados-local-pepper"),
            tasks,
            secrets,
        ))
    }

    /// Deployed environment (Lambda): AWS adapters chosen by configuration.
    pub async fn from_env() -> anyhow::Result<Self> {
        let cfg = Config::from_env();
        cfg.guard(chrono::Utc::now()).map_err(anyhow::Error::msg)?;
        let sdk = kabados_runtime::aws_config(&cfg).await;
        let db = Db::new(kabados_runtime::store(&cfg, Some(&sdk)).await?);
        let secrets = Secrets::new(Some(aws_sdk_secretsmanager::Client::new(&sdk)), cfg.env.clone());
        let authz: Arc<dyn Authorizer> = match &cfg.avp_policy_store_id {
            Some(id) => Arc::new(VerifiedPermissions::new(aws_sdk_verifiedpermissions::Client::new(&sdk), id.clone())),
            None => Arc::new(LocalCedar::new()?),
        };
        let sealer: Arc<dyn Sealer> = match &cfg.kms_key_id {
            Some(k) => Arc::new(KmsSealer::new(aws_sdk_kms::Client::new(&sdk), k.clone())),
            None if !cfg.is_prod() => Arc::new(LocalSealer::new(&format!("kabados-{}", cfg.env))),
            None => anyhow::bail!("KMS_KEY_ID is required in prod"),
        };
        let pepper = match secrets.get("pepper", "KABADOS_PEPPER").await {
            Ok(p) => p,
            Err(_) if !cfg.is_prod() => format!("kabados-{}-pepper", cfg.env),
            Err(e) => return Err(e),
        };
        let ix = Integrations::deployed(&cfg, &sdk, db.clone(), &secrets).await?;
        let tasks = Arc::new(SqsTaskQueue::from_env(aws_sdk_sqs::Client::new(&sdk)));
        let rt = crate::ai::runtime(&cfg, &secrets).await?;
        Ok(Self::new(cfg, db, authz, ix, sealer, Hasher::new(pepper), tasks, secrets).with_ai(Arc::new(AgentsAi::new(rt))))
    }

    /// Replaces the AI services (wired by binaries that link `kabados-agents`).
    pub fn with_ai(self, ai: Arc<dyn AiServices>) -> Self {
        let inner = Arc::try_unwrap(self.0).unwrap_or_else(|_| panic!("with_ai must be called before the state is shared"));
        Self(Arc::new(Inner { ai, ..inner }))
    }

    /// Taxonomy (seed + ops overrides), cached for five minutes per container.
    pub async fn taxonomy(&self) -> anyhow::Result<Arc<Taxonomy>> {
        if let Some((at, t)) = self.taxonomy.read().await.as_ref()
            && at.elapsed() < Duration::from_secs(300)
        {
            return Ok(t.clone());
        }
        let t = Arc::new(self.db.taxonomy().await?);
        *self.taxonomy.write().await = Some((Instant::now(), t.clone()));
        Ok(t)
    }

    pub async fn invalidate_taxonomy(&self) {
        *self.taxonomy.write().await = None;
    }
}
