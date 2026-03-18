use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{Context, Result, bail};
use serde::Serialize;

const DEFAULT_APP_NAME: &str = "DmxForge";
const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_DATABASE_URL: &str = "sqlite://data/dmxforge.db";
const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 5;
const DEFAULT_SESSION_COOKIE_NAME: &str = "dmxforge_session";
const DEFAULT_SESSION_TTL_HOURS: u64 = 24;
const DEFAULT_SECURE_COOKIES: bool = false;
const DEFAULT_PAYLOAD_LIMIT_KB: usize = 512;
const DEFAULT_SECRET_KEY: &str = "change-me-before-production-this-key-is-32b";

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    pub app_name: String,
    pub bind_address: String,
    pub port: u16,
    pub database_url: String,
    pub database_max_connections: u32,
    pub session_cookie_name: String,
    pub session_ttl_hours: u64,
    pub secure_cookies: bool,
    pub payload_limit_kb: usize,
    pub secret_key: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let config = Self {
            app_name: DEFAULT_APP_NAME.to_string(),
            bind_address: DEFAULT_BIND_ADDRESS.to_string(),
            port: DEFAULT_PORT,
            database_url: DEFAULT_DATABASE_URL.to_string(),
            database_max_connections: DEFAULT_DATABASE_MAX_CONNECTIONS,
            session_cookie_name: DEFAULT_SESSION_COOKIE_NAME.to_string(),
            session_ttl_hours: DEFAULT_SESSION_TTL_HOURS,
            secure_cookies: env_parse("SECURE_COOKIES", DEFAULT_SECURE_COOKIES)?,
            payload_limit_kb: DEFAULT_PAYLOAD_LIMIT_KB,
            secret_key: env_var("SECRET_KEY").unwrap_or_else(|| DEFAULT_SECRET_KEY.to_string()),
        };

        config.validate()?;
        Ok(config)
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::from_str(&format!("{}:{}", self.bind_address, self.port))
            .expect("validated bind address")
    }

    pub fn payload_limit_bytes(&self) -> usize {
        self.payload_limit_kb * 1024
    }

    pub fn static_dir(&self) -> PathBuf {
        env_var("DMXFORGE_STATIC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static"))
    }

    fn validate(&self) -> Result<()> {
        if self.app_name.trim().is_empty() {
            bail!("APP_NAME cannot be empty");
        }

        let _ = self.socket_addr();

        if self.database_max_connections == 0 {
            bail!("DATABASE_MAX_CONNECTIONS must be greater than 0");
        }

        if self.session_cookie_name.trim().is_empty() {
            bail!("SESSION_COOKIE_NAME cannot be empty");
        }

        if self.session_ttl_hours == 0 {
            bail!("SESSION_TTL_HOURS must be greater than 0");
        }

        if self.payload_limit_kb == 0 {
            bail!("PAYLOAD_LIMIT_KB must be greater than 0");
        }

        if self.secret_key.trim().len() < 32 {
            bail!("SECRET_KEY must contain at least 32 characters");
        }

        Ok(())
    }
}

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().map(|value| value.trim().to_string())
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env_var(key) {
        Some(raw) if !raw.is_empty() => raw
            .parse::<T>()
            .with_context(|| format!("failed to parse {key} from environment")),
        _ => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    fn base_config() -> AppConfig {
        AppConfig {
            app_name: "DmxForge".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 3000,
            database_url: "sqlite://data/dmxforge.db".to_string(),
            database_max_connections: 5,
            session_cookie_name: "dmxforge_session".to_string(),
            session_ttl_hours: 24,
            secure_cookies: false,
            payload_limit_kb: 512,
            secret_key: "0123456789abcdef0123456789abcdef".to_string(),
        }
    }

    #[test]
    fn rejects_short_secret_key() {
        let mut config = base_config();
        config.secret_key = "too-short".to_string();

        let error = config.validate().unwrap_err();
        assert!(error.to_string().contains("SECRET_KEY"));
    }

    #[test]
    fn accepts_valid_secret_key() {
        let mut config = base_config();
        config.secret_key = "0123456789abcdef0123456789abcdef".to_string();

        assert!(config.validate().is_ok());
    }
}
