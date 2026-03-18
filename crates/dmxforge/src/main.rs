use anyhow::{Result, bail};
use dmxforge::{config::AppConfig, init_tracing, migrate_only, run};

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    init_tracing()?;

    match std::env::args().nth(1).as_deref() {
        None => run(config).await,
        Some("--migrate-only") => migrate_only(config).await,
        Some(flag) => bail!("unsupported flag: {flag}"),
    }
}
