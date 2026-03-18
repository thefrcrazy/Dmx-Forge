use anyhow::{Context, Result, anyhow};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::Sha256;

use crate::db;
use super::model::Provider;

type HmacSha256 = Hmac<Sha256>;
type HmacSha1 = Hmac<Sha1>;

pub fn verify_source_signature(
    provider: Provider,
    source: &db::SourceRecord,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    let Some(secret) = source
        .webhook_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    match provider {
        Provider::Github => verify_github_signature(secret, headers, body),
        Provider::Gitlab => verify_gitlab_token(secret, headers),
        Provider::Gitea => verify_gitea_signature(secret, headers, body),
    }
}

fn verify_github_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> Result<()> {
    if let Some(signature) = header_value(headers, "x-hub-signature-256") {
        verify_prefixed_hmac_sha256(secret, body, &signature, "sha256=")?;
        return Ok(());
    }

    if let Some(signature) = header_value(headers, "x-hub-signature") {
        verify_prefixed_hmac_sha1(secret, body, &signature, "sha1=")?;
        return Ok(());
    }

    Err(anyhow!("missing GitHub signature header"))
}

fn verify_gitlab_token(secret: &str, headers: &HeaderMap) -> Result<()> {
    let Some(token) = header_value(headers, "x-gitlab-token") else {
        return Err(anyhow!("missing GitLab token header"));
    };

    if token == secret {
        Ok(())
    } else {
        Err(anyhow!("invalid GitLab token"))
    }
}

fn verify_gitea_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> Result<()> {
    let signature = header_value(headers, "x-gitea-signature")
        .or_else(|| header_value(headers, "x-gogs-signature"))
        .ok_or_else(|| anyhow!("missing Gitea signature header"))?;

    let expected = decode_hex(&signature)?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).context("failed to init Gitea HMAC")?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| anyhow!("invalid Gitea signature"))?;

    Ok(())
}

fn verify_prefixed_hmac_sha256(
    secret: &str,
    body: &[u8],
    signature: &str,
    prefix: &str,
) -> Result<()> {
    let encoded = signature
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("invalid GitHub SHA256 signature prefix"))?;
    let expected = decode_hex(encoded)?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .context("failed to init GitHub SHA256 HMAC")?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| anyhow!("invalid GitHub SHA256 signature"))?;

    Ok(())
}

fn verify_prefixed_hmac_sha1(
    secret: &str,
    body: &[u8],
    signature: &str,
    prefix: &str,
) -> Result<()> {
    let encoded = signature
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("invalid GitHub SHA1 signature prefix"))?;
    let expected = decode_hex(encoded)?;

    let mut mac =
        HmacSha1::new_from_slice(secret.as_bytes()).context("failed to init GitHub SHA1 HMAC")?;
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| anyhow!("invalid GitHub SHA1 signature"))?;

    Ok(())
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>> {
    hex::decode(value.trim()).with_context(|| format!("invalid hex signature: {value}"))
}

pub fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

pub fn headers_to_json(headers: &HeaderMap) -> Value {
    let pairs = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                Value::String(value.to_str().unwrap_or("<binary>").to_string()),
            )
        })
        .collect::<serde_json::Map<String, Value>>();

    json!(pairs)
}

pub fn header_map_from_json(raw_headers: &str) -> Result<HeaderMap> {
    let pairs = serde_json::from_str::<Map<String, Value>>(raw_headers)
        .context("failed to deserialize raw header JSON")?;
    let mut headers = HeaderMap::new();

    for (name, value) in pairs {
        let Some(raw_value) = value.as_str() else {
            continue;
        };

        let header_name = HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid header name in replay payload: {name}"))?;
        let header_value = HeaderValue::from_str(raw_value)
            .with_context(|| format!("invalid header value for replay header: {name}"))?;
        headers.insert(header_name, header_value);
    }

    Ok(headers)
}

pub fn markdown_link(label: &str, url: &str) -> String {
    format!("[{}]({})", label.replace(']', "\\]"), url)
}

pub fn shorten_commit_id(value: &str) -> String {
    value.chars().take(7).collect()
}

pub fn parse_embed_color(value: &str) -> Option<u64> {
    let normalized = value.trim().trim_start_matches('#');
    u64::from_str_radix(normalized, 16).ok()
}

pub fn parse_filter_list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn branch_matches_pattern(branch: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        branch.starts_with(prefix)
    } else {
        branch == pattern
    }
}
