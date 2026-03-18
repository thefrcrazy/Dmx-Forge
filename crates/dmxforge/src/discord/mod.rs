use anyhow::{Context, Result, bail};
use minijinja::{AutoEscape, Environment};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Clone)]
pub struct DiscordTemplateEngine {
    sample_payload: Value,
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub template: String,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    pub rendered: String,
    pub sample_payload: Value,
}

impl DiscordTemplateEngine {
    pub fn new() -> Self {
        Self {
            sample_payload: sample_preview_payload(),
        }
    }

    pub fn render_preview(&self, template: &str) -> Result<String> {
        self.render_template(template, &self.sample_payload)
    }

    pub fn render_template(&self, template: &str, payload: &Value) -> Result<String> {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|_| AutoEscape::None);
        env.add_template("preview", template)
            .context("failed to register preview template")?;

        env.get_template("preview")
            .context("failed to retrieve preview template")?
            .render(payload.clone())
            .context("failed to render preview template")
    }

    pub fn sample_payload(&self) -> Value {
        self.sample_payload.clone()
    }
}

pub fn validate_webhook_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).context("failed to parse Discord webhook URL")?;

    if url.scheme() != "https" {
        bail!("Discord webhook URL must use https");
    }

    let Some(host) = url.host_str() else {
        bail!("Discord webhook URL must include a host");
    };

    if !matches!(
        host,
        "discord.com" | "ptb.discord.com" | "canary.discord.com"
    ) {
        bail!("Discord webhook host is not allowed");
    }

    if !url.path().starts_with("/api/webhooks/") {
        bail!("Discord webhook path must start with /api/webhooks/");
    }

    if !url.username().is_empty() || url.password().is_some() {
        bail!("Discord webhook URL must not include user info");
    }

    Ok(url)
}

pub fn sample_preview_payload() -> Value {
    json!({
        "provider": "github",
        "event_type": "push",
        "repository": {
            "name": "dmxforge",
            "full_name": "acme/dmxforge",
            "url": "https://github.com/acme/dmxforge"
        },
        "actor": {
            "name": "Acme",
            "username": "acme"
        },
        "branch": "main",
        "compare_url": "https://github.com/acme/dmxforge/compare/abc1234...def5678",
        "commit_count": 3,
        "commits": [
            {
                "id": "abc1234",
                "message": "Bootstrap Axum, Askama and SQLx foundation"
            },
            {
                "id": "bcd2345",
                "message": "Add dashboard shell and preview endpoint"
            },
            {
                "id": "cde3456",
                "message": "Prepare Docker and Traefik deployment files"
            }
        ],
        "timestamp": "2026-03-13T00:00:00Z"
    })
}

#[cfg(test)]
mod tests {
    use super::validate_webhook_url;

    #[test]
    fn accepts_official_discord_hosts() {
        let url = validate_webhook_url("https://discord.com/api/webhooks/1/token").unwrap();
        assert_eq!(url.host_str(), Some("discord.com"));
    }

    #[test]
    fn rejects_unknown_hosts() {
        let error = validate_webhook_url("https://example.com/api/webhooks/1/token").unwrap_err();
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn rejects_non_https_webhook_urls() {
        let error = validate_webhook_url("http://discord.com/api/webhooks/1/token").unwrap_err();
        assert!(error.to_string().contains("https"));
    }
}
