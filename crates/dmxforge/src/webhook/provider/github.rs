use serde_json::Value;
use crate::webhook::model::{UnifiedActor, UnifiedCommit, UnifiedEvent, UnifiedRepository};
use super::{strip_ref_prefix, timestamp_from_candidates};
use crate::webhook::utils::shorten_commit_id;

pub fn normalize(event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match event_type {
        "push" => Some(build_event(
            "github",
            "push",
            repository(payload),
            actor(payload),
            branch(payload),
            compare_url(payload),
            commits(payload.pointer("/commits").and_then(Value::as_array)),
            Some(push_title(payload)),
            payload
                .pointer("/head_commit/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            payload
                .pointer("/compare")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| repository(payload).url.clone()),
            timestamp_from_candidates(
                payload,
                &["/head_commit/timestamp", "/repository/updated_at", "/after"],
            ),
            payload.clone(),
        )),
        "pull_request" => Some(build_event(
            "github",
            "pull_request",
            repository(payload),
            actor(payload),
            payload
                .pointer("/pull_request/head/ref")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            Vec::new(),
            payload
                .pointer("/pull_request/title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/pull_request/body")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            pull_request_status(payload),
            payload
                .pointer("/pull_request/html_url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(
                payload,
                &["/pull_request/updated_at", "/pull_request/created_at"],
            ),
            payload.clone(),
        )),
        "issues" => Some(build_event(
            "github",
            "issues",
            repository(payload),
            actor(payload),
            None,
            None,
            Vec::new(),
            payload
                .pointer("/issue/title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/issue/body")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/action")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    payload
                        .pointer("/issue/state")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            payload
                .pointer("/issue/html_url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(payload, &["/issue/updated_at", "/issue/created_at"]),
            payload.clone(),
        )),
        "release" => Some(build_event(
            "github",
            "release",
            repository(payload),
            actor(payload),
            payload
                .pointer("/release/target_commitish")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            Vec::new(),
            payload
                .pointer("/release/name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    payload
                        .pointer("/release/tag_name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            payload
                .pointer("/release/body")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/action")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/release/html_url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(payload, &["/release/published_at", "/release/created_at"]),
            payload.clone(),
        )),
        _ => None,
    }
}

fn build_event(
    provider: &str,
    event_type: &str,
    repository: UnifiedRepository,
    actor: UnifiedActor,
    branch: Option<String>,
    compare_url: Option<String>,
    commits: Vec<UnifiedCommit>,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    url: Option<String>,
    timestamp: String,
    metadata: Value,
) -> UnifiedEvent {
    let commit_count = commits.len();

    UnifiedEvent {
        provider: provider.to_string(),
        event_type: event_type.to_string(),
        repository,
        actor,
        branch,
        compare_url,
        commit_count,
        commits,
        title,
        description,
        status,
        url,
        timestamp,
        metadata,
    }
}

fn repository(payload: &Value) -> UnifiedRepository {
    UnifiedRepository {
        name: payload
            .pointer("/repository/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        full_name: payload
            .pointer("/repository/full_name")
            .and_then(Value::as_str)
            .or_else(|| payload.pointer("/repository/name").and_then(Value::as_str))
            .unwrap_or("unknown")
            .to_string(),
        url: payload
            .pointer("/repository/html_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

fn actor(payload: &Value) -> UnifiedActor {
    let username = payload
        .pointer("/sender/login")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/pusher/name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    let name = payload
        .pointer("/sender/login")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/pusher/name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    UnifiedActor {
        name,
        username,
        url: payload
            .pointer("/sender/html_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        avatar_url: payload
            .pointer("/sender/avatar_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

fn branch(payload: &Value) -> Option<String> {
    payload
        .pointer("/ref")
        .and_then(Value::as_str)
        .map(strip_ref_prefix)
}

fn compare_url(payload: &Value) -> Option<String> {
    payload
        .pointer("/compare")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .pointer("/compare_url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn commits(commits: Option<&Vec<Value>>) -> Vec<UnifiedCommit> {
    commits
        .into_iter()
        .flat_map(|items| items.iter())
        .map(|commit| {
            let id = commit
                .pointer("/id")
                .and_then(Value::as_str)
                .or_else(|| commit.pointer("/sha").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();

            UnifiedCommit {
                short_id: shorten_commit_id(&id),
                id,
                message: commit
                    .pointer("/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message")
                    .to_string(),
                url: commit
                    .pointer("/url")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                author_name: commit
                    .pointer("/author/name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        commit
                            .pointer("/author_name")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    }),
            }
        })
        .collect()
}

fn pull_request_status(payload: &Value) -> Option<String> {
    if payload
        .pointer("/pull_request/merged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Some("merged".to_string());
    }

    payload
        .pointer("/action")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn push_title(payload: &Value) -> String {
    let repo_name = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .unwrap_or("repository");
    format!("Push received for {repo_name}")
}
