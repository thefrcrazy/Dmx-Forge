use std::{fs, path::Path, str::FromStr};

use anyhow::{Context, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::config::AppConfig;

pub mod delivery;
pub mod settings;
pub mod stats;
pub mod user;
pub mod webhook;

// Re-export commonly used types and functions
pub use delivery::*;
pub use settings::*;
pub use stats::*;
pub use user::*;
pub use webhook::*;

pub type DbPool = SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn connect(config: &AppConfig) -> Result<DbPool> {
    ensure_database_parent(&config.database_url)?;

    let options = SqliteConnectOptions::from_str(&config.database_url)
        .with_context(|| format!("invalid DATABASE_URL: {}", config.database_url))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect_with(options)
        .await
        .context("failed to connect to sqlite database")?;

    MIGRATOR
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    settings::seed_settings(&pool, config).await?;
    webhook::seed_default_templates(&pool).await?;

    Ok(pool)
}

pub async fn ping(pool: &DbPool) -> Result<()> {
    sqlx::query("SELECT 1")
        .execute(pool)
        .await
        .context("database ping failed")?;

    Ok(())
}

fn ensure_database_parent(database_url: &str) -> Result<()> {
    let Some(raw_path) = sqlite_path(database_url) else {
        return Ok(());
    };

    let normalized = raw_path.split('?').next().unwrap_or(raw_path);
    let path = Path::new(normalized);
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create sqlite parent directory {parent:?}"))?;
    }

    Ok(())
}

fn sqlite_path(database_url: &str) -> Option<&str> {
    if database_url == "sqlite::memory:" || database_url.starts_with("sqlite:file::memory:") {
        return None;
    }

    database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
}

pub(crate) fn as_sqlite_bool(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub(crate) fn generate_token() -> String {
    use uuid::Uuid;
    Uuid::new_v4().simple().to_string()
}
