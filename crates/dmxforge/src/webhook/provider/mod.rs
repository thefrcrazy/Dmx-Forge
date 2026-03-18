use serde_json::Value;
use axum::http::HeaderMap;

use super::model::{Provider, UnifiedEvent};

pub mod github;
pub mod gitlab;
pub mod gitea;

pub fn normalize_event(provider: Provider, event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match provider {
        Provider::Github => github::normalize(event_type, payload),
        Provider::Gitlab => gitlab::normalize(event_type, payload),
        Provider::Gitea => gitea::normalize(event_type, payload),
    }
}

pub fn event_type_from_headers(provider: Provider, headers: &HeaderMap) -> Option<String> {
    let header_name = match provider {
        Provider::Github => "x-github-event",
        Provider::Gitlab => "x-gitlab-event",
        Provider::Gitea => "x-gitea-event-type",
    };

    headers
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .map(|value| normalize_event_type(provider, value))
}

pub fn infer_event_type(provider: Provider, payload: &Value) -> Option<String> {
    match provider {
        Provider::Github | Provider::Gitea => {
            if payload.get("pull_request").is_some() {
                Some("pull_request".to_string())
            } else if payload.get("issue").is_some() {
                Some("issues".to_string())
            } else if payload.get("release").is_some() {
                Some("release".to_string())
            } else if payload.get("commits").is_some() || payload.get("head_commit").is_some() {
                Some("push".to_string())
            } else {
                None
            }
        }
        Provider::Gitlab => payload
            .pointer("/object_kind")
            .and_then(Value::as_str)
            .map(|value| normalize_event_type(provider, value)),
    }
}

pub fn normalize_event_type(provider: Provider, raw: &str) -> String {
    let value = raw.trim().to_lowercase();
    match provider {
        Provider::Github | Provider::Gitea => value,
        Provider::Gitlab => match value.as_str() {
            "push hook" | "push" => "push".to_string(),
            "tag push hook" | "tag_push" | "tag push" => "tag_push".to_string(),
            "merge request hook" | "merge_request" | "merge request" => "merge_request".to_string(),
            "pipeline hook" | "pipeline" => "pipeline".to_string(),
            "release hook" | "release" => "release".to_string(),
            other => other.replace(' ', "_"),
        },
    }
}

pub(crate) fn strip_ref_prefix(value: &str) -> String {
    value
        .strip_prefix("refs/heads/")
        .or_else(|| value.strip_prefix("refs/tags/"))
        .unwrap_or(value)
        .to_string()
}

pub(crate) fn timestamp_from_candidates(payload: &Value, pointers: &[&str]) -> String {
    for pointer in pointers {
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }

    chrono::Utc::now().to_rfc3339()
}
