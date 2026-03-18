use serde_json::Value;
use crate::webhook::model::{UnifiedActor, UnifiedCommit, UnifiedEvent, UnifiedRepository};
use super::{strip_ref_prefix, timestamp_from_candidates};
use crate::webhook::utils::shorten_commit_id;

pub fn normalize(event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match event_type {
        "push" | "tag_push" => Some(build_event(
            "gitlab",
            event_type,
            repository(payload),
            actor(payload),
            branch(payload),
            payload
                .pointer("/compare")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            commits(payload.pointer("/commits").and_then(Value::as_array)),
            Some(push_title(payload)),
            payload
                .pointer("/commits/0/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            payload
                .pointer("/compare")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(payload, &["/event_created_at", "/commits/0/timestamp"]),
            payload.clone(),
        )),
        "merge_request" => Some(build_event(
            "gitlab",
            "merge_request",
            repository(payload),
            actor(payload),
            payload
                .pointer("/object_attributes/source_branch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            Vec::new(),
            payload
                .pointer("/object_attributes/title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/object_attributes/description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            merge_request_status(payload),
            payload
                .pointer("/object_attributes/url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(
                payload,
                &[
                    "/object_attributes/updated_at",
                    "/object_attributes/created_at",
                ],
            ),
            payload.clone(),
        )),
        "pipeline" => Some(build_event(
            "gitlab",
            "pipeline",
            repository(payload),
            actor(payload),
            payload
                .pointer("/object_attributes/ref")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            Vec::new(),
            payload
                .pointer("/object_attributes/name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    payload
                        .pointer("/object_attributes/id")
                        .and_then(Value::as_i64)
                        .map(|value| format!("Pipeline #{value}"))
                }),
            None,
            payload
                .pointer("/object_attributes/status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/object_attributes/url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(payload, &["/object_attributes/created_at"]),
            payload.clone(),
        )),
        "release" => Some(build_event(
            "gitlab",
            "release",
            repository(payload),
            actor(payload),
            payload
                .pointer("/tag")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            Vec::new(),
            payload
                .pointer("/name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload
                .pointer("/description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            payload
                .pointer("/url")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            timestamp_from_candidates(payload, &["/released_at", "/created_at"]),
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
            .pointer("/project/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        full_name: payload
            .pointer("/project/path_with_namespace")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        url: payload
            .pointer("/project/web_url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

fn actor(payload: &Value) -> UnifiedActor {
    let username = payload
        .pointer("/user_username")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/user/name").and_then(Value::as_str))
        .or_else(|| payload.pointer("/user_name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    let name = payload
        .pointer("/user_name")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/user/name").and_then(Value::as_str))
        .or_else(|| payload.pointer("/user_username").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    UnifiedActor {
        name,
        username,
        url: payload
            .pointer("/user_url")
            .and_then(Value::as_str)
            .or_else(|| payload.pointer("/user/web_url").and_then(Value::as_str))
            .map(ToOwned::to_owned),
        avatar_url: payload
            .pointer("/user_avatar")
            .and_then(Value::as_str)
            .or_else(|| payload.pointer("/user/avatar_url").and_then(Value::as_str))
            .map(ToOwned::to_owned),
    }
}

fn branch(payload: &Value) -> Option<String> {
    payload
        .pointer("/ref")
        .and_then(Value::as_str)
        .map(strip_ref_prefix)
        .or_else(|| {
            payload
                .pointer("/object_attributes/ref")
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

fn merge_request_status(payload: &Value) -> Option<String> {
    if let Some(state) = payload
        .pointer("/object_attributes/state")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Some(state.to_string());
    }

    payload
        .pointer("/object_attributes/action")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn push_title(payload: &Value) -> String {
    let repo_name = payload
        .pointer("/project/path_with_namespace")
        .and_then(Value::as_str)
        .unwrap_or("repository");
    format!("Push received for {repo_name}")
}
