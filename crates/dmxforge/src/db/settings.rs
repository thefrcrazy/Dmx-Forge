use anyhow::{Context, Result};
use sqlx::{FromRow, SqlitePool};

use crate::config::AppConfig;

#[derive(Debug, Clone, FromRow)]
pub struct AppSettingItem {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

pub async fn list_app_settings(pool: &SqlitePool) -> Result<Vec<AppSettingItem>> {
    let items = sqlx::query_as::<_, AppSettingItem>(
        r#"
        SELECT key, value, updated_at
        FROM app_settings
        ORDER BY key ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list app settings")?;

    Ok(items)
}

pub async fn seed_settings(pool: &SqlitePool, config: &AppConfig) -> Result<()> {
    let payload_limit = config.payload_limit_kb.to_string();
    let settings = [
        ("instance_name", config.app_name.as_str()),
        ("payload_limit_kb", payload_limit.as_str()),
        ("session_cookie_name", config.session_cookie_name.as_str()),
    ];

    for (key, value) in settings {
        sqlx::query(
            r#"
            INSERT INTO app_settings (key, value, updated_at)
            VALUES (?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .with_context(|| format!("failed to seed app setting {key}"))?;
    }

    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind("public_base_url")
        .execute(pool)
        .await
        .context("failed to remove deprecated app setting public_base_url")?;

    Ok(())
}
