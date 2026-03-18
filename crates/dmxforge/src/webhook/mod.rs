use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue},
    routing::post,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    AppError, AppState,
    db::{self, NewDiscordMessageAttempt, NewWebhookDelivery},
};

type HmacSha256 = Hmac<Sha256>;
type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Clone, Copy)]
enum Provider {
    Github,
    Gitlab,
    Gitea,
}

impl Provider {
    fn parse(input: &str) -> Option<Self> {
        match input {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "gitea" | "forgejo" => Some(Self::Gitea),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitea => "gitea",
        }
    }
}

#[derive(Debug, Serialize)]
struct AcceptedWebhook {
    delivery_id: Uuid,
    status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct UnifiedEvent {
    provider: String,
    event_type: String,
    repository: UnifiedRepository,
    actor: UnifiedActor,
    branch: Option<String>,
    compare_url: Option<String>,
    commit_count: usize,
    commits: Vec<UnifiedCommit>,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    url: Option<String>,
    timestamp: String,
    metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
struct UnifiedRepository {
    name: String,
    full_name: String,
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UnifiedActor {
    name: String,
    username: String,
    url: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UnifiedCommit {
    id: String,
    short_id: String,
    message: String,
    url: Option<String>,
    author_name: Option<String>,
}

#[derive(Debug)]
struct MatchedRoute<'a> {
    rule: &'a db::RoutingRuleListItem,
    destination: &'a db::DestinationListItem,
    template: &'a db::MessageTemplateListItem,
}

#[derive(Debug, Clone)]
pub struct RouteWebhookTestRequest {
    pub provider: String,
    pub event_type: String,
    pub repository: String,
    pub branch: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/webhooks/{provider}/{token}", post(receive_webhook))
        .with_state(state)
}

pub async fn enqueue_replay_from_delivery(
    state: Arc<AppState>,
    original_delivery_id: &str,
) -> Result<Uuid> {
    let replay = db::find_delivery_replay_record(&state.db, original_delivery_id, None)
        .await?
        .ok_or_else(|| anyhow!("delivery not found"))?;
    let source_id = replay
        .source_id
        .as_deref()
        .ok_or_else(|| anyhow!("delivery has no associated source"))?;
    let source = db::find_source_record_by_id(&state.db, source_id)
        .await?
        .ok_or_else(|| anyhow!("source not found for replay"))?;
    let provider =
        Provider::parse(&replay.provider).ok_or_else(|| anyhow!("unsupported webhook provider"))?;
    let headers = header_map_from_json(&replay.raw_headers)?;

    let delivery_id = db::save_incoming_delivery(
        &state.db,
        NewWebhookDelivery {
            source_id: source.id.clone(),
            provider: replay.provider,
            event_type: replay.event_type,
            repository: replay.repository,
            branch: replay.branch,
            raw_headers: replay.raw_headers.clone(),
            raw_payload: replay.raw_payload.clone(),
        },
    )
    .await?;

    let state_for_task = state.clone();
    let state_for_error = state.clone();
    tokio::spawn(async move {
        if let Err(error) = process_delivery(
            state_for_task,
            provider,
            source,
            headers,
            replay.raw_payload,
            delivery_id,
            false,
        )
        .await
        {
            tracing::error!(
                delivery_id = %delivery_id,
                error = %error,
                "failed to replay webhook delivery"
            );

            if let Err(mark_error) =
                db::mark_delivery_failed(&state_for_error.db, delivery_id, &error.to_string(), None)
                    .await
            {
                tracing::error!(
                    delivery_id = %delivery_id,
                    error = %mark_error,
                    "failed to mark replayed webhook delivery as failed"
                );
            }
        }
    });

    Ok(delivery_id)
}

pub async fn send_test_route_webhook(
    state: &Arc<AppState>,
    destination: &db::DestinationListItem,
    template: &db::MessageTemplateListItem,
    request: RouteWebhookTestRequest,
) -> Result<()> {
    let event = build_test_route_event(&request);
    let event_payload =
        serde_json::to_value(&event).context("failed to serialize test webhook event")?;
    let rendered_body = state
        .discord
        .render_template(&template.body_template, &event_payload)
        .context("failed to render route template")?;
    let request_payload = build_discord_payload(&event, template, &rendered_body);
    let response = state
        .http_client
        .post(destination.webhook_url.clone())
        .json(&request_payload)
        .send()
        .await
        .context("failed to reach Discord")?;
    let status = response.status();
    let response_body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let detail = response_body.trim();
        if detail.is_empty() {
            return Err(anyhow!(
                "Discord rejected the test webhook with status {status}."
            ));
        }

        return Err(anyhow!(
            "Discord rejected the test webhook with status {status}: {detail}"
        ));
    }

    Ok(())
}

async fn receive_webhook(
    State(state): State<Arc<AppState>>,
    Path((provider, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<AcceptedWebhook>, AppError> {
    let provider = Provider::parse(provider.as_str())
        .ok_or_else(|| AppError::bad_request("unsupported webhook provider"))?;

    let source = db::find_source_by_provider_token(&state.db, provider.as_str(), &token)
        .await?
        .ok_or_else(|| AppError::not_found("unknown webhook source"))?;

    if source.is_active == 0 {
        return Err(AppError::bad_request("webhook source is disabled"));
    }

    let payload = parse_payload(&body);
    let event_type = event_type_from_headers(provider, &headers)
        .or_else(|| infer_event_type(provider, &payload));
    let repository = repository_name(&payload);
    let branch = branch_name(provider, &payload);
    let raw_headers =
        serde_json::to_string(&headers_to_json(&headers)).context("failed to serialize headers")?;

    let delivery_id = db::save_incoming_delivery(
        &state.db,
        NewWebhookDelivery {
            source_id: source.id.clone(),
            provider: provider.as_str().to_string(),
            event_type,
            repository,
            branch,
            raw_headers,
            raw_payload: body.clone(),
        },
    )
    .await?;

    let state_for_task = state.clone();
    let state_for_error = state.clone();
    let headers_for_task = headers.clone();
    tokio::spawn(async move {
        if let Err(error) = process_delivery(
            state_for_task,
            provider,
            source,
            headers_for_task,
            body,
            delivery_id,
            true,
        )
        .await
        {
            tracing::error!(
                delivery_id = %delivery_id,
                error = %error,
                "failed to process webhook delivery"
            );

            if let Err(mark_error) =
                db::mark_delivery_failed(&state_for_error.db, delivery_id, &error.to_string(), None)
                    .await
            {
                tracing::error!(
                    delivery_id = %delivery_id,
                    error = %mark_error,
                    "failed to mark webhook delivery as failed after processing error"
                );
            }
        }
    });

    Ok(Json(AcceptedWebhook {
        delivery_id,
        status: "accepted",
    }))
}

async fn process_delivery(
    state: Arc<AppState>,
    provider: Provider,
    source: db::SourceRecord,
    headers: HeaderMap,
    body: String,
    delivery_id: Uuid,
    verify_signature: bool,
) -> Result<()> {
    if verify_signature {
        if let Err(error) = verify_source_signature(provider, &source, &headers, body.as_bytes()) {
            db::mark_delivery_failed(&state.db, delivery_id, &error.to_string(), None).await?;
            return Ok(());
        }
    }

    let payload: Value = match serde_json::from_str(&body) {
        Ok(payload) => payload,
        Err(error) => {
            db::mark_delivery_failed(
                &state.db,
                delivery_id,
                &format!("invalid JSON payload: {error}"),
                None,
            )
            .await?;
            return Ok(());
        }
    };

    let event_type = event_type_from_headers(provider, &headers)
        .or_else(|| infer_event_type(provider, &payload))
        .unwrap_or_else(|| "unknown".to_string());

    let Some(event) = normalize_event(provider, &event_type, &payload) else {
        db::mark_delivery_skipped(
            &state.db,
            delivery_id,
            &format!("unsupported event type: {event_type}"),
        )
        .await?;
        return Ok(());
    };

    let normalized_event =
        serde_json::to_string(&event).context("failed to serialize normalized event")?;

    if let Some(reason) = source_filter_reason(&source, &event) {
        db::mark_delivery_skipped_with_event(
            &state.db,
            delivery_id,
            &reason,
            Some(&normalized_event),
        )
        .await?;
        return Ok(());
    }

    let rules = db::list_routing_rules(&state.db).await?;
    let destinations = db::list_destinations(&state.db).await?;
    let templates = db::list_message_templates(&state.db).await?;
    let matched_routes = match_routes(&source, &event, &rules, &destinations, &templates);

    if matched_routes.is_empty() {
        db::mark_delivery_skipped_with_event(
            &state.db,
            delivery_id,
            "no active routing rule matched this event",
            Some(&normalized_event),
        )
        .await?;
        return Ok(());
    }

    let event_payload =
        serde_json::to_value(&event).context("failed to convert normalized event")?;
    let mut sent_count = 0usize;
    let mut failed_count = 0usize;

    for route in matched_routes {
        let outcome = send_route_message(
            &state,
            delivery_id,
            &event,
            &event_payload,
            route.destination,
            route.template,
            route.rule,
        )
        .await?;

        if outcome.sent {
            sent_count += 1;
        }
        if outcome.failed {
            failed_count += 1;
        }
    }

    if sent_count > 0 {
        let failure_reason = if failed_count > 0 {
            Some(format!(
                "partial failure: {failed_count} of {} attempts failed",
                sent_count + failed_count
            ))
        } else {
            None
        };

        db::mark_delivery_processed(
            &state.db,
            delivery_id,
            &normalized_event,
            failure_reason.as_deref(),
        )
        .await?;
    } else {
        db::mark_delivery_failed(
            &state.db,
            delivery_id,
            &format!("all {} Discord deliveries failed", failed_count),
            Some(&normalized_event),
        )
        .await?;
    }

    Ok(())
}

#[derive(Debug)]
struct RouteOutcome {
    sent: bool,
    failed: bool,
}

async fn send_route_message(
    state: &Arc<AppState>,
    delivery_id: Uuid,
    event: &UnifiedEvent,
    event_payload: &Value,
    destination: &db::DestinationListItem,
    template: &db::MessageTemplateListItem,
    rule: &db::RoutingRuleListItem,
) -> Result<RouteOutcome> {
    let rendered_body = match state
        .discord
        .render_template(&template.body_template, event_payload)
    {
        Ok(value) => value,
        Err(error) => {
            db::save_discord_message_attempt(
                &state.db,
                NewDiscordMessageAttempt {
                    delivery_id: delivery_id.to_string(),
                    destination_id: Some(destination.id.clone()),
                    request_payload: json!({
                        "render_error": error.to_string(),
                        "rule_id": rule.id,
                        "template_id": template.id,
                    })
                    .to_string(),
                    response_status: None,
                    response_body: Some(error.to_string()),
                    status: "failed".to_string(),
                },
            )
            .await?;

            return Ok(RouteOutcome {
                sent: false,
                failed: true,
            });
        }
    };

    let request_payload = build_discord_payload(event, template, &rendered_body);
    let request_payload_json = serde_json::to_string(&request_payload)
        .context("failed to serialize Discord request payload")?;

    let response = state
        .http_client
        .post(destination.webhook_url.clone())
        .json(&request_payload)
        .send()
        .await;

    match response {
        Ok(response) => {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            let sent = status.is_success();

            db::save_discord_message_attempt(
                &state.db,
                NewDiscordMessageAttempt {
                    delivery_id: delivery_id.to_string(),
                    destination_id: Some(destination.id.clone()),
                    request_payload: request_payload_json,
                    response_status: Some(i64::from(status.as_u16())),
                    response_body: Some(response_body),
                    status: if sent { "sent" } else { "failed" }.to_string(),
                },
            )
            .await?;

            Ok(RouteOutcome {
                sent,
                failed: !sent,
            })
        }
        Err(error) => {
            db::save_discord_message_attempt(
                &state.db,
                NewDiscordMessageAttempt {
                    delivery_id: delivery_id.to_string(),
                    destination_id: Some(destination.id.clone()),
                    request_payload: request_payload_json,
                    response_status: None,
                    response_body: Some(error.to_string()),
                    status: "failed".to_string(),
                },
            )
            .await?;

            Ok(RouteOutcome {
                sent: false,
                failed: true,
            })
        }
    }
}

fn verify_source_signature(
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

fn source_filter_reason(source: &db::SourceRecord, event: &UnifiedEvent) -> Option<String> {
    if let Some(filter) = source
        .repository_filter
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !filter.eq_ignore_ascii_case(&event.repository.full_name) {
            return Some(format!(
                "repository '{}' does not match source filter '{}'",
                event.repository.full_name, filter
            ));
        }
    }

    let allowed_events = parse_filter_list(source.allowed_events.as_deref());
    if !allowed_events.is_empty()
        && !allowed_events
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&event.event_type))
    {
        return Some(format!(
            "event '{}' is not allowed by source filter",
            event.event_type
        ));
    }

    let allowed_branches = parse_filter_list(source.allowed_branches.as_deref());
    if !allowed_branches.is_empty() {
        let Some(branch) = event.branch.as_deref() else {
            return Some("event has no branch but source requires branch filters".to_string());
        };

        if !allowed_branches
            .iter()
            .any(|pattern| branch_matches_pattern(branch, pattern))
        {
            return Some(format!(
                "branch '{}' is not allowed by source filter",
                branch
            ));
        }
    }

    None
}

fn match_routes<'a>(
    source: &db::SourceRecord,
    event: &UnifiedEvent,
    rules: &'a [db::RoutingRuleListItem],
    destinations: &'a [db::DestinationListItem],
    templates: &'a [db::MessageTemplateListItem],
) -> Vec<MatchedRoute<'a>> {
    let active_destinations = destinations
        .iter()
        .filter(|item| item.is_active == 1)
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let active_templates = templates
        .iter()
        .filter(|item| item.is_active == 1)
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();

    rules
        .iter()
        .filter(|rule| rule.is_active == 1)
        .filter(|rule| route_matches(rule, source, event))
        .filter_map(|rule| {
            let destination = active_destinations.get(rule.destination_id.as_str())?;
            let template = active_templates.get(rule.template_id.as_str())?;

            Some(MatchedRoute {
                rule,
                destination,
                template,
            })
        })
        .collect()
}

fn route_matches(
    rule: &db::RoutingRuleListItem,
    source: &db::SourceRecord,
    event: &UnifiedEvent,
) -> bool {
    if let Some(source_id) = rule.source_id.as_deref() {
        if source_id != source.id {
            return false;
        }
    }

    if let Some(provider_filter) = rule.provider_filter.as_deref() {
        if !provider_filter.eq_ignore_ascii_case(&event.provider) {
            return false;
        }
    }

    if let Some(event_filter) = rule.event_type_filter.as_deref() {
        if !event_filter.eq_ignore_ascii_case(&event.event_type) {
            return false;
        }
    }

    if let Some(repository_filter) = rule.repository_filter.as_deref() {
        if !repository_filter.eq_ignore_ascii_case(&event.repository.full_name) {
            return false;
        }
    }

    if let Some(branch_prefix_filter) = rule.branch_prefix_filter.as_deref() {
        let Some(branch) = event.branch.as_deref() else {
            return false;
        };

        if !branch.starts_with(branch_prefix_filter) {
            return false;
        }
    }

    if let Some(skip_keyword) = rule.skip_keyword.as_deref() {
        let keyword = skip_keyword.to_lowercase();
        if event
            .commits
            .iter()
            .any(|commit| commit.message.to_lowercase().contains(&keyword))
        {
            return false;
        }
    }

    true
}

fn build_discord_payload(
    event: &UnifiedEvent,
    template: &db::MessageTemplateListItem,
    rendered_body: &str,
) -> Value {
    let is_compact = template.format_style.eq_ignore_ascii_case("compact");
    let split_commit_embed = should_split_push_commit_embed(event, template);
    let mut root = Map::new();
    root.insert("allowed_mentions".to_string(), json!({ "parse": [] }));

    if let Some(username) = template
        .username_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        root.insert("username".to_string(), Value::String(username.to_string()));
    }

    if let Some(avatar_url) = template
        .avatar_url_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        root.insert(
            "avatar_url".to_string(),
            Value::String(avatar_url.to_string()),
        );
    }

    let mut embed = Map::new();
    embed.insert(
        "title".to_string(),
        Value::String(if is_compact {
            compact_embed_title(event, rendered_body)
        } else {
            event
                .title
                .clone()
                .unwrap_or_else(|| format!("{} · {}", event.event_type, event.repository.full_name))
        }),
    );
    if let Some(description) = compact_embed_description(event, rendered_body, is_compact) {
        embed.insert("description".to_string(), Value::String(description));
    }

    if template.show_repo_link == 1 {
        let primary_url = if is_compact {
            compact_primary_url(event)
        } else {
            event.url.clone().or_else(|| event.repository.url.clone())
        };
        if let Some(url) = primary_url.filter(|value| !value.is_empty()) {
            embed.insert("url".to_string(), Value::String(url));
        }
    }

    if let Some(color) = effective_embed_color(event, template) {
        embed.insert("color".to_string(), Value::Number(color.into()));
    }

    if !event.actor.name.is_empty() {
        let mut author = Map::new();
        author.insert(
            "name".to_string(),
            Value::String(actor_display_name(&event.actor)),
        );
        if let Some(url) = event.actor.url.as_deref().filter(|value| !value.is_empty()) {
            author.insert("url".to_string(), Value::String(url.to_string()));
        }
        if template.show_avatar == 1 {
            if let Some(icon_url) = event
                .actor
                .avatar_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    template
                        .avatar_url_override
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                })
            {
                author.insert("icon_url".to_string(), Value::String(icon_url.to_string()));
            }
        }
        embed.insert("author".to_string(), Value::Object(author));
    }

    if template.show_timestamp == 1 {
        embed.insert(
            "timestamp".to_string(),
            Value::String(event.timestamp.clone()),
        );
    }

    if let Some(footer_text) = template
        .footer_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !is_seeded_footer_text(value))
    {
        embed.insert(
            "footer".to_string(),
            json!({
                "text": footer_text
            }),
        );
    }

    let mut fields = Vec::new();

    if template.show_repo_link == 1 {
        if let Some(url) = event
            .repository
            .url
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            fields.push(json!({
                "name": "Repository",
                "value": markdown_link(&event.repository.full_name, url),
                "inline": true,
            }));
        }
    }

    if template.show_branch == 1 {
        if let Some(branch) = event.branch.as_deref().filter(|value| !value.is_empty()) {
            fields.push(json!({
                "name": if is_pull_request_event(event) { "Source branch" } else { "Branch" },
                "value": branch,
                "inline": true,
            }));
        }
    }

    if is_pull_request_event(event) {
        if let Some(target_branch) = pull_request_target_branch(event) {
            fields.push(json!({
                "name": "Target branch",
                "value": target_branch,
                "inline": true,
            }));
        }
    }

    if template.show_status_badge == 1 {
        if let Some(status) = event_status_label(event) {
            fields.push(json!({
                "name": "Status",
                "value": status,
                "inline": true,
            }));
        }
    }

    if !is_compact {
        if let Some(compare_url) = event
            .compare_url
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            fields.push(json!({
                "name": "Compare",
                "value": markdown_link("Open compare", compare_url),
                "inline": false,
            }));
        }
    }

    if template.show_commits == 1 && !event.commits.is_empty() && !split_commit_embed {
        let value = event
            .commits
            .iter()
            .take(5)
            .map(format_commit_line)
            .collect::<Vec<_>>()
            .join("\n");

        fields.push(json!({
            "name": "Commits",
            "value": value,
            "inline": false,
        }));
    }

    if !fields.is_empty() {
        embed.insert("fields".to_string(), Value::Array(fields));
    }

    let mut embeds = vec![Value::Object(embed)];
    if split_commit_embed {
        embeds.push(Value::Object(build_commit_detail_embed(event, template)));
    }

    root.insert("embeds".to_string(), Value::Array(embeds));

    Value::Object(root)
}

fn compact_embed_title(event: &UnifiedEvent, rendered_body: &str) -> String {
    if event.event_type.eq_ignore_ascii_case("push") {
        let actor = actor_display_name(&event.actor);
        return match push_kind(event) {
            PushKind::Merge => {
                let target = default_branch_name(event)
                    .or_else(|| event.branch.as_deref())
                    .unwrap_or("default branch");
                format!("{actor} merged into {target}")
            }
            PushKind::Sync => {
                let target = pull_request_base_branch(event)
                    .or_else(|| default_branch_name(event).map(ToOwned::to_owned))
                    .unwrap_or_else(|| "base branch".to_string());
                format!("{actor} synced PR branch with {target}")
            }
            PushKind::PullRequestBranch => format!("{actor} updated PR branch"),
            PushKind::Normal => {
                let noun = if event.commit_count > 1 {
                    "commits"
                } else {
                    "commit"
                };
                format!("{actor} pushed {} {noun}", event.commit_count)
            }
        };
    }

    if is_pull_request_event(event) {
        let actor = actor_display_name(&event.actor);
        let status = compact_pull_request_action(event);
        let number = pull_request_number(event)
            .map(|value| format!("PR #{value}"))
            .unwrap_or_else(|| "pull request".to_string());
        return format!("{actor} {status} {number}");
    }

    if event.event_type.eq_ignore_ascii_case("release") {
        let actor = actor_display_name(&event.actor);
        let status = event_status_label(event).unwrap_or_else(|| "updated".to_string());
        let release_name = event
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("release");
        return format!("{actor} {status} {release_name}");
    }

    let inline = rendered_body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !inline.is_empty() {
        return inline;
    }

    event
        .title
        .clone()
        .unwrap_or_else(|| format!("{} · {}", event.event_type, event.repository.full_name))
}

fn is_seeded_footer_text(value: &str) -> bool {
    matches!(
        value,
        "Compact activity" | "Detailed timeline" | "Release signal" | "Failure watch"
    )
}

fn compact_embed_description(
    event: &UnifiedEvent,
    rendered_body: &str,
    is_compact: bool,
) -> Option<String> {
    if !is_compact {
        return (!rendered_body.trim().is_empty()).then(|| rendered_body.to_string());
    }

    if is_pull_request_event(event) {
        return event
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
    }

    None
}

fn compact_primary_url(event: &UnifiedEvent) -> Option<String> {
    if event.event_type.eq_ignore_ascii_case("push") {
        event.repository.url.clone().or_else(|| event.url.clone())
    } else {
        event.url.clone().or_else(|| event.repository.url.clone())
    }
}

fn should_split_push_commit_embed(
    event: &UnifiedEvent,
    template: &db::MessageTemplateListItem,
) -> bool {
    template.show_commits == 1
        && !event.commits.is_empty()
        && matches!(push_kind(event), PushKind::Sync)
}

fn build_commit_detail_embed(
    event: &UnifiedEvent,
    template: &db::MessageTemplateListItem,
) -> Map<String, Value> {
    let mut embed = Map::new();

    embed.insert(
        "title".to_string(),
        Value::String(match push_kind(event) {
            PushKind::Merge => "Merged commits".to_string(),
            PushKind::Sync => "Included commits".to_string(),
            _ => "Commits".to_string(),
        }),
    );

    let description = event
        .commits
        .iter()
        .take(5)
        .map(format_commit_line)
        .collect::<Vec<_>>()
        .join("\n");
    embed.insert("description".to_string(), Value::String(description));

    if let Some(color) = effective_embed_color(event, template) {
        embed.insert("color".to_string(), Value::Number(color.into()));
    }

    if template.show_timestamp == 1 {
        embed.insert(
            "timestamp".to_string(),
            Value::String(event.timestamp.clone()),
        );
    }

    embed
}

fn effective_embed_color(
    event: &UnifiedEvent,
    template: &db::MessageTemplateListItem,
) -> Option<u64> {
    if uses_seeded_system_color(template) {
        return event_accent_color(event)
            .or_else(|| template.embed_color.as_deref().and_then(parse_embed_color));
    }

    template.embed_color.as_deref().and_then(parse_embed_color)
}

fn uses_seeded_system_color(template: &db::MessageTemplateListItem) -> bool {
    match template.format_style.as_str() {
        "compact" => color_matches_seed(template.embed_color.as_deref(), "3B82F6"),
        "detailed" => color_matches_seed(template.embed_color.as_deref(), "10B981"),
        "release" => color_matches_seed(template.embed_color.as_deref(), "F59E0B"),
        "alert" => color_matches_seed(template.embed_color.as_deref(), "EF4444"),
        _ => false,
    }
}

fn color_matches_seed(value: Option<&str>, expected: &str) -> bool {
    value
        .map(|item| {
            item.trim()
                .trim_start_matches('#')
                .eq_ignore_ascii_case(expected)
        })
        .unwrap_or(true)
}

fn event_accent_color(event: &UnifiedEvent) -> Option<u64> {
    let color = if event.event_type.eq_ignore_ascii_case("push")
        || event.event_type.eq_ignore_ascii_case("tag_push")
    {
        match push_kind(event) {
            PushKind::Merge => "#16A34A",
            PushKind::Sync => "#0891B2",
            PushKind::PullRequestBranch => "#0EA5A4",
            PushKind::Normal => "#2563EB",
        }
    } else if is_pull_request_event(event) {
        if is_merged_pull_request(event) {
            "#16A34A"
        } else if event_status_label(event)
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("closed"))
            .unwrap_or(false)
        {
            "#DC2626"
        } else {
            "#7C3AED"
        }
    } else if event.event_type.eq_ignore_ascii_case("release") {
        "#D97706"
    } else if event.event_type.eq_ignore_ascii_case("pipeline") {
        match event_status_label(event)
            .as_deref()
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("success") | Some("passed") => "#16A34A",
            Some("failed") | Some("error") => "#DC2626",
            Some("canceled") | Some("cancelled") => "#D97706",
            _ => "#2563EB",
        }
    } else {
        return None;
    };

    parse_embed_color(color)
}

fn is_pull_request_event(event: &UnifiedEvent) -> bool {
    event.event_type.eq_ignore_ascii_case("pull_request")
        || event.event_type.eq_ignore_ascii_case("merge_request")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PushKind {
    Normal,
    PullRequestBranch,
    Sync,
    Merge,
}

fn push_kind(event: &UnifiedEvent) -> PushKind {
    if is_merge_push(event) {
        PushKind::Merge
    } else if is_sync_push(event) {
        PushKind::Sync
    } else if is_pull_request_branch_push(event) {
        PushKind::PullRequestBranch
    } else {
        PushKind::Normal
    }
}

fn push_branch_name(event: &UnifiedEvent) -> Option<&str> {
    event
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_non_default_branch_push(event: &UnifiedEvent) -> bool {
    if !(event.event_type.eq_ignore_ascii_case("push")
        || event.event_type.eq_ignore_ascii_case("tag_push"))
    {
        return false;
    }

    matches!(
        (push_branch_name(event), default_branch_name(event)),
        (Some(branch), Some(default_branch)) if branch != default_branch
    )
}

fn is_pull_request_branch_push(event: &UnifiedEvent) -> bool {
    if !is_non_default_branch_push(event) {
        return false;
    }

    if head_commit_message(event)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
        .map(|value| value.starts_with("merge pull request #"))
        .unwrap_or(false)
    {
        return true;
    }

    matches!(
        (push_branch_name(event), pull_request_base_branch(event).as_deref()),
        (Some(branch), Some(base_branch)) if branch != base_branch
    )
}

fn is_sync_push(event: &UnifiedEvent) -> bool {
    if !is_non_default_branch_push(event) {
        return false;
    }

    let Some(message) = head_commit_message(event).map(|value| value.to_ascii_lowercase()) else {
        return false;
    };
    let Some(base_branch) = pull_request_base_branch(event)
        .or_else(|| default_branch_name(event).map(ToOwned::to_owned))
        .map(|value| value.to_ascii_lowercase())
    else {
        return false;
    };

    message.starts_with("merge ")
        && (message.contains(&format!("merge branch '{}'", base_branch))
            || message.contains(&format!("merge branch \"{}\"", base_branch))
            || message.contains(&format!("origin/{}", base_branch))
            || message.contains(&format!("into {}", base_branch)))
}

fn is_merge_push(event: &UnifiedEvent) -> bool {
    if !(event.event_type.eq_ignore_ascii_case("push")
        || event.event_type.eq_ignore_ascii_case("tag_push"))
    {
        return false;
    }

    let Some(default_branch) = default_branch_name(event) else {
        return false;
    };
    let Some(branch) = event
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    if branch != default_branch {
        return false;
    }

    let Some(message) = head_commit_message(event).map(|value| value.to_ascii_lowercase()) else {
        return false;
    };

    message.starts_with("merge pull request #")
        || message.starts_with("merge branch ")
        || looks_like_squash_merge_message(&message)
}

fn looks_like_squash_merge_message(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.ends_with(')') && trimmed.contains(" (#")
}

fn head_commit_message(event: &UnifiedEvent) -> Option<String> {
    event
        .metadata
        .pointer("/head_commit/message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .description
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn default_branch_name(event: &UnifiedEvent) -> Option<&str> {
    event
        .metadata
        .pointer("/repository/default_branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn pull_request_base_branch(event: &UnifiedEvent) -> Option<String> {
    [
        "/base_ref",
        "/pull_request/base/ref",
        "/object_attributes/target_branch",
        "/merge_request/target_branch",
        "/target_branch",
    ]
    .iter()
    .find_map(|pointer| event.metadata.pointer(pointer).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(strip_ref_prefix)
}

fn is_merged_pull_request(event: &UnifiedEvent) -> bool {
    event_status_label(event)
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("merged"))
        .unwrap_or(false)
}

fn event_status_label(event: &UnifiedEvent) -> Option<String> {
    event
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| match push_kind(event) {
            PushKind::Merge => Some("merged".to_string()),
            PushKind::Sync => Some("synced".to_string()),
            _ => None,
        })
}

fn compact_pull_request_action(event: &UnifiedEvent) -> &'static str {
    match event_status_label(event)
        .as_deref()
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("merged") => "merged",
        Some("opened") | Some("open") => "opened",
        Some("reopened") => "reopened",
        Some("closed") => "closed",
        Some("draft") => "drafted",
        _ => "updated",
    }
}

fn pull_request_number(event: &UnifiedEvent) -> Option<i64> {
    event
        .metadata
        .pointer("/pull_request/number")
        .and_then(Value::as_i64)
        .or_else(|| event.metadata.pointer("/number").and_then(Value::as_i64))
        .or_else(|| {
            event
                .metadata
                .pointer("/object_attributes/iid")
                .and_then(Value::as_i64)
        })
        .or_else(|| {
            event
                .metadata
                .pointer("/object_attributes/id")
                .and_then(Value::as_i64)
        })
}

fn pull_request_target_branch(event: &UnifiedEvent) -> Option<String> {
    event
        .metadata
        .pointer("/pull_request/base/ref")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .metadata
                .pointer("/object_attributes/target_branch")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn build_test_route_event(request: &RouteWebhookTestRequest) -> UnifiedEvent {
    let branch = request
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let repository_url = test_repository_url(&request.provider, &request.repository);
    let compare_url = repository_url
        .clone()
        .map(|url| format!("{url}/compare/abc1234...def5678",));
    let repo_name = request
        .repository
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(request.repository.as_str())
        .to_string();
    let actor_username = request
        .repository
        .split('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("acme");
    let actor_name = test_actor_name(actor_username);

    UnifiedEvent {
        provider: request.provider.clone(),
        event_type: request.event_type.clone(),
        repository: UnifiedRepository {
            name: repo_name,
            full_name: request.repository.clone(),
            url: repository_url.clone(),
        },
        actor: UnifiedActor {
            name: actor_name.clone(),
            username: actor_username.to_string(),
            url: test_actor_url(&request.provider, actor_username),
            avatar_url: test_actor_avatar_url(&request.provider, actor_username),
        },
        branch,
        compare_url: compare_url.clone(),
        commit_count: 2,
        commits: vec![
            UnifiedCommit {
                id: "abc1234".to_string(),
                short_id: "abc1234".to_string(),
                message: "Manual route webhook test".to_string(),
                url: compare_url.clone(),
                author_name: Some(actor_name.clone()),
            },
            UnifiedCommit {
                id: "def5678".to_string(),
                short_id: "def5678".to_string(),
                message: "Discord destination connectivity check".to_string(),
                url: compare_url.clone(),
                author_name: Some(actor_name.clone()),
            },
        ],
        title: Some(format!(
            "{} test · {}",
            request.event_type, request.repository
        )),
        description: Some("Manual webhook test triggered from the routing UI.".to_string()),
        status: Some("success".to_string()),
        url: compare_url.or(repository_url),
        timestamp: Utc::now().to_rfc3339(),
        metadata: json!({
            "manual_test": true,
        }),
    }
}

fn test_repository_url(provider: &str, repository: &str) -> Option<String> {
    match Provider::parse(provider) {
        Some(Provider::Github) => Some(format!("https://github.com/{repository}")),
        Some(Provider::Gitlab) => Some(format!("https://gitlab.com/{repository}")),
        Some(Provider::Gitea) | None => None,
    }
}

fn test_actor_url(provider: &str, username: &str) -> Option<String> {
    match Provider::parse(provider) {
        Some(Provider::Github) => Some(format!("https://github.com/{username}")),
        Some(Provider::Gitlab) => Some(format!("https://gitlab.com/{username}")),
        Some(Provider::Gitea) | None => None,
    }
}

fn test_actor_avatar_url(provider: &str, username: &str) -> Option<String> {
    match Provider::parse(provider) {
        Some(Provider::Github) => Some(format!("https://github.com/{username}.png?size=128")),
        Some(Provider::Gitlab) => None,
        Some(Provider::Gitea) | None => None,
    }
}

fn test_actor_name(username: &str) -> String {
    let username = username.trim();

    if username.is_empty() {
        return "Acme".to_string();
    }

    let mut chars = username.chars();
    match chars.next() {
        Some(first) => {
            let mut result = first.to_uppercase().collect::<String>();
            result.push_str(chars.as_str());
            result
        }
        None => "Acme".to_string(),
    }
}

fn normalize_event(provider: Provider, event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match provider {
        Provider::Github => normalize_github_event(event_type, payload),
        Provider::Gitlab => normalize_gitlab_event(event_type, payload),
        Provider::Gitea => normalize_gitea_event(event_type, payload),
    }
}

fn normalize_github_event(event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match event_type {
        "push" => Some(build_event(
            "github",
            "push",
            repository_from_github_like(payload),
            actor_from_github(payload),
            branch_name(Provider::Github, payload),
            compare_url(payload),
            commits_from_array(payload.pointer("/commits").and_then(Value::as_array)),
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
                .or_else(|| repository_from_github_like(payload).url.clone()),
            timestamp_from_candidates(
                payload,
                &["/head_commit/timestamp", "/repository/updated_at", "/after"],
            ),
            payload.clone(),
        )),
        "pull_request" => Some(build_event(
            "github",
            "pull_request",
            repository_from_github_like(payload),
            actor_from_github(payload),
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
            github_pull_request_status(payload),
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
            repository_from_github_like(payload),
            actor_from_github(payload),
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
            repository_from_github_like(payload),
            actor_from_github(payload),
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

fn normalize_gitlab_event(event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match event_type {
        "push" | "tag_push" => Some(build_event(
            "gitlab",
            event_type,
            repository_from_gitlab(payload),
            actor_from_gitlab(payload),
            branch_name(Provider::Gitlab, payload),
            payload
                .pointer("/compare")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            commits_from_array(payload.pointer("/commits").and_then(Value::as_array)),
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
            repository_from_gitlab(payload),
            actor_from_gitlab(payload),
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
            gitlab_merge_request_status(payload),
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
            repository_from_gitlab(payload),
            actor_from_gitlab(payload),
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
            repository_from_gitlab(payload),
            actor_from_gitlab(payload),
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

fn normalize_gitea_event(event_type: &str, payload: &Value) -> Option<UnifiedEvent> {
    match event_type {
        "push" => Some(build_event(
            "gitea",
            "push",
            repository_from_github_like(payload),
            actor_from_gitea(payload),
            branch_name(Provider::Gitea, payload),
            compare_url(payload),
            commits_from_array(payload.pointer("/commits").and_then(Value::as_array)),
            Some(push_title(payload)),
            payload
                .pointer("/head_commit/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            None,
            compare_url(payload),
            timestamp_from_candidates(
                payload,
                &["/head_commit/timestamp", "/repository/updated_at"],
            ),
            payload.clone(),
        )),
        "pull_request" => Some(build_event(
            "gitea",
            "pull_request",
            repository_from_github_like(payload),
            actor_from_gitea(payload),
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
            gitea_pull_request_status(payload),
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
            "gitea",
            "issues",
            repository_from_github_like(payload),
            actor_from_gitea(payload),
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
            "gitea",
            "release",
            repository_from_github_like(payload),
            actor_from_gitea(payload),
            payload
                .pointer("/release/tag_name")
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

fn repository_from_github_like(payload: &Value) -> UnifiedRepository {
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

fn repository_from_gitlab(payload: &Value) -> UnifiedRepository {
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

fn actor_from_github(payload: &Value) -> UnifiedActor {
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

fn actor_from_gitlab(payload: &Value) -> UnifiedActor {
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

fn actor_from_gitea(payload: &Value) -> UnifiedActor {
    let username = payload
        .pointer("/sender/login")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/pusher/name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    let name = payload
        .pointer("/sender/full_name")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/sender/login").and_then(Value::as_str))
        .or_else(|| payload.pointer("/pusher/full_name").and_then(Value::as_str))
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

fn actor_display_name(actor: &UnifiedActor) -> String {
    let name = actor.name.trim();
    let username = actor.username.trim();

    if username.is_empty() || username.eq_ignore_ascii_case(name) {
        if name.is_empty() {
            "unknown".to_string()
        } else {
            name.to_string()
        }
    } else if name.is_empty() {
        format!("@{username}")
    } else {
        format!("{name} (@{username})")
    }
}

fn markdown_link(label: &str, url: &str) -> String {
    format!("[{}]({})", label.replace(']', "\\]"), url)
}

fn format_commit_line(commit: &UnifiedCommit) -> String {
    let commit_ref = commit
        .url
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|url| markdown_link(&commit.short_id, url))
        .unwrap_or_else(|| format!("`{}`", commit.short_id));

    if let Some(author_name) = commit
        .author_name
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        format!("- {commit_ref} {} ({author_name})", commit.message)
    } else {
        format!("- {commit_ref} {}", commit.message)
    }
}

fn commits_from_array(commits: Option<&Vec<Value>>) -> Vec<UnifiedCommit> {
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

fn branch_name(provider: Provider, payload: &Value) -> Option<String> {
    match provider {
        Provider::Github | Provider::Gitea => payload
            .pointer("/ref")
            .and_then(Value::as_str)
            .map(strip_ref_prefix),
        Provider::Gitlab => payload
            .pointer("/ref")
            .and_then(Value::as_str)
            .map(strip_ref_prefix)
            .or_else(|| {
                payload
                    .pointer("/object_attributes/ref")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            }),
    }
}

fn repository_name(payload: &Value) -> Option<String> {
    payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .pointer("/project/path_with_namespace")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .pointer("/repository/name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
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

fn github_pull_request_status(payload: &Value) -> Option<String> {
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

fn gitlab_merge_request_status(payload: &Value) -> Option<String> {
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

fn gitea_pull_request_status(payload: &Value) -> Option<String> {
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
    let branch = repository_name(payload).unwrap_or_else(|| "repository".to_string());
    format!("Push received for {branch}")
}

fn timestamp_from_candidates(payload: &Value, pointers: &[&str]) -> String {
    for pointer in pointers {
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }

    chrono::Utc::now().to_rfc3339()
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn event_type_from_headers(provider: Provider, headers: &HeaderMap) -> Option<String> {
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

fn infer_event_type(provider: Provider, payload: &Value) -> Option<String> {
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

fn normalize_event_type(provider: Provider, raw: &str) -> String {
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

fn parse_payload(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or(Value::Null)
}

fn headers_to_json(headers: &HeaderMap) -> Value {
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

fn parse_filter_list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn branch_matches_pattern(branch: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        branch.starts_with(prefix)
    } else {
        branch == pattern
    }
}

fn strip_ref_prefix(value: &str) -> String {
    value
        .strip_prefix("refs/heads/")
        .or_else(|| value.strip_prefix("refs/tags/"))
        .unwrap_or(value)
        .to_string()
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    hex::decode(value.trim()).with_context(|| format!("invalid hex signature: {value}"))
}

fn header_map_from_json(raw_headers: &str) -> Result<HeaderMap> {
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

fn shorten_commit_id(value: &str) -> String {
    value.chars().take(7).collect()
}

fn parse_embed_color(value: &str) -> Option<u64> {
    let normalized = value.trim().trim_start_matches('#');
    u64::from_str_radix(normalized, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_headers(name: &str, value: &str) -> HeaderMap {
        use axum::http::header::HeaderName;

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        headers
    }

    #[test]
    fn verifies_github_sha256_signature() {
        let body = br#"{"hello":"world"}"#;
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let headers = github_headers("x-hub-signature-256", &signature);
        assert!(verify_github_signature("secret", &headers, body).is_ok());
    }

    #[test]
    fn verifies_github_sha1_fallback_signature() {
        let body = br#"{"hello":"world"}"#;
        let mut mac = HmacSha1::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = format!("sha1={}", hex::encode(mac.finalize().into_bytes()));

        let headers = github_headers("x-hub-signature", &signature);
        assert!(verify_github_signature("secret", &headers, body).is_ok());
    }

    #[test]
    fn verifies_gitlab_token_header() {
        let headers = github_headers("x-gitlab-token", "secret");
        assert!(verify_gitlab_token("secret", &headers).is_ok());
    }

    #[test]
    fn verifies_gitea_signature() {
        let body = br#"{"hello":"world"}"#;
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        let headers = github_headers("x-gitea-signature", &signature);
        assert!(verify_gitea_signature("secret", &headers, body).is_ok());
    }

    #[test]
    fn source_branch_filter_supports_wildcard_prefix() {
        let source = db::SourceRecord {
            id: "source-1".to_string(),
            name: "Main source".to_string(),
            provider: "github".to_string(),
            user_id: None,
            webhook_secret: None,
            repository_filter: None,
            allowed_branches: Some("main\nrelease/*".to_string()),
            allowed_events: Some("push".to_string()),
            is_active: 1,
        };
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: None,
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: None,
                avatar_url: None,
            },
            branch: Some("release/1.0".to_string()),
            compare_url: None,
            commit_count: 0,
            commits: Vec::new(),
            title: None,
            description: None,
            status: None,
            url: None,
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: Value::Null,
        };

        assert_eq!(source_filter_reason(&source, &event), None);
    }

    #[test]
    fn route_skip_keyword_blocks_match() {
        let source = db::SourceRecord {
            id: "source-1".to_string(),
            name: "Main source".to_string(),
            provider: "github".to_string(),
            user_id: None,
            webhook_secret: None,
            repository_filter: None,
            allowed_branches: None,
            allowed_events: None,
            is_active: 1,
        };
        let rule = db::RoutingRuleListItem {
            id: "rule-1".to_string(),
            name: "Rule".to_string(),
            user_id: None,
            source_id: Some("source-1".to_string()),
            source_name: Some("Main source".to_string()),
            destination_id: "dest-1".to_string(),
            destination_name: "Discord".to_string(),
            template_id: "tpl-1".to_string(),
            template_name: "Compact".to_string(),
            provider_filter: Some("github".to_string()),
            event_type_filter: Some("push".to_string()),
            branch_prefix_filter: Some("main".to_string()),
            repository_filter: Some("acme/dmxforge".to_string()),
            skip_keyword: Some("[skip-discord]".to_string()),
            sort_order: 0,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: None,
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: None,
                avatar_url: None,
            },
            branch: Some("main".to_string()),
            compare_url: None,
            commit_count: 1,
            commits: vec![UnifiedCommit {
                id: "abcdef123456".to_string(),
                short_id: "abcdef1".to_string(),
                message: "Ship feature [skip-discord]".to_string(),
                url: None,
                author_name: None,
            }],
            title: None,
            description: None,
            status: None,
            url: None,
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: Value::Null,
        };

        assert!(!route_matches(&rule, &source, &event));
    }

    #[test]
    fn normalizes_github_push_event() {
        let payload = json!({
            "ref": "refs/heads/main",
            "repository": {
                "name": "dmxforge",
                "full_name": "acme/dmxforge",
                "html_url": "https://github.com/acme/dmxforge"
            },
            "sender": {
                "login": "acme",
                "html_url": "https://github.com/acme",
                "avatar_url": "https://avatars.githubusercontent.com/u/1?v=4"
            },
            "compare": "https://github.com/acme/dmxforge/compare/a...b",
            "head_commit": {
                "message": "Ship pipeline",
                "timestamp": "2026-03-13T00:00:00Z"
            },
            "commits": [
                {
                    "id": "abcdef1234567",
                    "message": "Ship pipeline",
                    "url": "https://github.com/acme/dmxforge/commit/abcdef1234567",
                    "author": { "name": "Acme" }
                }
            ]
        });

        let event = normalize_github_event("push", &payload).unwrap();
        assert_eq!(event.repository.full_name, "acme/dmxforge");
        assert_eq!(event.branch.as_deref(), Some("main"));
        assert_eq!(event.commit_count, 1);
        assert_eq!(event.commits[0].short_id, "abcdef1");
        assert_eq!(event.actor.url.as_deref(), Some("https://github.com/acme"));
        assert_eq!(
            event.actor.avatar_url.as_deref(),
            Some("https://avatars.githubusercontent.com/u/1?v=4")
        );
    }

    #[test]
    fn normalizes_github_merged_pull_request_event() {
        let payload = json!({
            "action": "closed",
            "number": 42,
            "repository": {
                "name": "dmxforge",
                "full_name": "acme/dmxforge",
                "html_url": "https://github.com/acme/dmxforge"
            },
            "sender": {
                "login": "acme",
                "html_url": "https://github.com/acme",
                "avatar_url": "https://avatars.githubusercontent.com/u/1?v=4"
            },
            "pull_request": {
                "number": 42,
                "title": "Refactor Discord compact embed",
                "body": "Refresh the webhook output.",
                "html_url": "https://github.com/acme/dmxforge/pull/42",
                "merged": true,
                "head": {
                    "ref": "feature/compact"
                },
                "base": {
                    "ref": "main"
                },
                "updated_at": "2026-03-13T00:00:00Z"
            }
        });

        let event = normalize_github_event("pull_request", &payload).unwrap();
        assert_eq!(event.event_type, "pull_request");
        assert_eq!(event.status.as_deref(), Some("merged"));
        assert_eq!(event.branch.as_deref(), Some("feature/compact"));
        assert_eq!(
            event.url.as_deref(),
            Some("https://github.com/acme/dmxforge/pull/42")
        );
    }

    #[test]
    fn normalizes_gitlab_push_event() {
        let payload = json!({
            "object_kind": "push",
            "ref": "refs/heads/main",
            "project": {
                "name": "dmxforge",
                "path_with_namespace": "acme/dmxforge",
                "web_url": "https://gitlab.example/acme/dmxforge"
            },
            "user_name": "Acme",
            "user_username": "acme",
            "compare": "https://gitlab.example/acme/dmxforge/-/compare/a...b",
            "event_created_at": "2026-03-13T00:00:00Z",
            "commits": [
                {
                    "id": "abcdef1234567",
                    "message": "Ship webhook pipeline",
                    "url": "https://gitlab.example/acme/dmxforge/-/commit/abcdef1234567",
                    "author": { "name": "Acme" }
                }
            ]
        });

        let event = normalize_gitlab_event("push", &payload).unwrap();
        assert_eq!(event.event_type, "push");
        assert_eq!(event.repository.full_name, "acme/dmxforge");
        assert_eq!(event.branch.as_deref(), Some("main"));
        assert_eq!(event.commit_count, 1);
    }

    #[test]
    fn normalizes_gitea_release_event() {
        let payload = json!({
            "repository": {
                "name": "dmxforge",
                "full_name": "acme/dmxforge",
                "html_url": "https://gitea.example/acme/dmxforge"
            },
            "sender": {
                "login": "acme",
                "full_name": "Acme"
            },
            "release": {
                "name": "v1.0.0",
                "tag_name": "v1.0.0",
                "body": "First release",
                "html_url": "https://gitea.example/acme/dmxforge/releases/tag/v1.0.0",
                "published_at": "2026-03-13T00:00:00Z"
            },
            "action": "published"
        });

        let event = normalize_gitea_event("release", &payload).unwrap();
        assert_eq!(event.event_type, "release");
        assert_eq!(event.title.as_deref(), Some("v1.0.0"));
        assert_eq!(event.status.as_deref(), Some("published"));
    }

    #[test]
    fn compact_payload_formats_pull_request_event() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "pull_request".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("feature/compact".to_string()),
            compare_url: None,
            commit_count: 0,
            commits: Vec::new(),
            title: Some("Refactor Discord compact embed".to_string()),
            description: Some("Refresh the webhook output.".to_string()),
            status: Some("merged".to_string()),
            url: Some("https://github.com/acme/dmxforge/pull/42".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "pull_request": {
                    "number": 42,
                    "base": {
                        "ref": "main"
                    }
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-1".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();
        let title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();
        let description = payload
            .pointer("/embeds/0/description")
            .and_then(Value::as_str)
            .unwrap();
        let source_branch_label = payload
            .pointer("/embeds/0/fields/1/name")
            .and_then(Value::as_str)
            .unwrap();
        let target_branch_label = payload
            .pointer("/embeds/0/fields/2/name")
            .and_then(Value::as_str)
            .unwrap();
        let status_value = payload
            .pointer("/embeds/0/fields/3/value")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(color, parse_embed_color("#16A34A").unwrap());
        assert_eq!(title, "Acme merged PR #42");
        assert_eq!(description, "Refactor Discord compact embed");
        assert_eq!(source_branch_label, "Source branch");
        assert_eq!(target_branch_label, "Target branch");
        assert_eq!(status_value, "merged");
    }

    #[test]
    fn detailed_payload_uses_event_colors_for_pull_requests() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "pull_request".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("feature/compact".to_string()),
            compare_url: None,
            commit_count: 0,
            commits: Vec::new(),
            title: Some("Refactor Discord compact embed".to_string()),
            description: Some("Refresh the webhook output.".to_string()),
            status: Some("opened".to_string()),
            url: Some("https://github.com/acme/dmxforge/pull/42".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "pull_request": {
                    "number": 42,
                    "base": {
                        "ref": "main"
                    }
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-2".to_string(),
            name: "Detailed".to_string(),
            user_id: None,
            format_style: "detailed".to_string(),
            body_template: String::new(),
            embed_color: Some("#10B981".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "Detailed body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();
        let source_branch_label = payload
            .pointer("/embeds/0/fields/1/name")
            .and_then(Value::as_str)
            .unwrap();
        let target_branch_label = payload
            .pointer("/embeds/0/fields/2/name")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(color, parse_embed_color("#7C3AED").unwrap());
        assert_eq!(source_branch_label, "Source branch");
        assert_eq!(target_branch_label, "Target branch");
    }

    #[test]
    fn compact_payload_uses_distinct_color_for_pull_request_branch_push() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("feature/compact".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 2,
            commits: Vec::new(),
            title: Some("Push".to_string()),
            description: None,
            status: None,
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "base_ref": "refs/heads/main",
                "repository": {
                    "default_branch": "main"
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-3".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();

        assert_eq!(color, parse_embed_color("#0EA5A4").unwrap());
    }

    #[test]
    fn compact_payload_keeps_feature_branch_push_as_normal_push_without_pr_context() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("feature/compact".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 2,
            commits: Vec::new(),
            title: Some("Push".to_string()),
            description: Some("Regular feature branch push".to_string()),
            status: None,
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "repository": {
                    "default_branch": "main"
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-3b".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();
        let title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(color, parse_embed_color("#2563EB").unwrap());
        assert_eq!(title, "Acme pushed 2 commits");
    }

    #[test]
    fn compact_payload_detects_merge_pushes() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("main".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 1,
            commits: vec![UnifiedCommit {
                id: "abcdef123456".to_string(),
                short_id: "abcdef1".to_string(),
                message: "Merge branch payload".to_string(),
                url: Some("https://github.com/acme/dmxforge/commit/abcdef123456".to_string()),
                author_name: Some("Acme".to_string()),
            }],
            title: Some("Push".to_string()),
            description: Some("Merge pull request #42 from acme/feature-branch".to_string()),
            status: None,
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "repository": {
                    "default_branch": "main"
                },
                "head_commit": {
                    "message": "Merge pull request #42 from acme/feature-branch"
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-4".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();
        let title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();
        let status = payload
            .pointer("/embeds/0/fields/2/value")
            .and_then(Value::as_str)
            .unwrap();
        let commit_field = payload
            .pointer("/embeds/0/fields/3/value")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(color, parse_embed_color("#16A34A").unwrap());
        assert_eq!(title, "Acme merged into main");
        assert_eq!(status, "merged");
        assert_eq!(
            payload
                .pointer("/embeds")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert!(commit_field.contains("[abcdef1]("));
    }

    #[test]
    fn compact_payload_detects_sync_pushes() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("feature/compact".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 1,
            commits: vec![UnifiedCommit {
                id: "fedcba654321".to_string(),
                short_id: "fedcba6".to_string(),
                message: "Merge branch 'main' into feature/compact".to_string(),
                url: Some("https://github.com/acme/dmxforge/commit/fedcba654321".to_string()),
                author_name: Some("Acme".to_string()),
            }],
            title: Some("Push".to_string()),
            description: Some("Merge branch 'main' into feature/compact".to_string()),
            status: None,
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "repository": {
                    "default_branch": "main"
                },
                "head_commit": {
                    "message": "Merge branch 'main' into feature/compact"
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-5".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();
        let title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();
        let status = payload
            .pointer("/embeds/0/fields/2/value")
            .and_then(Value::as_str)
            .unwrap();
        let commit_embed_title = payload
            .pointer("/embeds/1/title")
            .and_then(Value::as_str)
            .unwrap();
        let commit_embed_description = payload
            .pointer("/embeds/1/description")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(color, parse_embed_color("#0891B2").unwrap());
        assert_eq!(title, "Acme synced PR branch with main");
        assert_eq!(status, "synced");
        assert_eq!(commit_embed_title, "Included commits");
        assert!(commit_embed_description.contains("[fedcba6]("));
    }

    #[test]
    fn compact_payload_treats_non_default_merge_pr_push_as_pr_branch_update() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Contributor".to_string(),
                username: "contributor".to_string(),
                url: Some("https://github.com/contributor".to_string()),
                avatar_url: None,
            },
            branch: Some("feature-a".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 4,
            commits: vec![UnifiedCommit {
                id: "a997a866233285d25360ff5825f13de6dd3af875".to_string(),
                short_id: "a997a86".to_string(),
                message: "Merge pull request #53 from acme/feature-b".to_string(),
                url: Some(
                    "https://github.com/acme/dmxforge/commit/a997a866233285d25360ff5825f13de6dd3af875"
                        .to_string(),
                ),
                author_name: Some("Acme".to_string()),
            }],
            title: Some("Push".to_string()),
            description: Some(
                "Merge pull request #53 from acme/feature-b\n\nFeature B".to_string(),
            ),
            status: None,
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: json!({
                "base_ref": "refs/heads/feature-a",
                "repository": {
                    "default_branch": "main"
                },
                "head_commit": {
                    "message": "Merge pull request #53 from acme/feature-b"
                }
            }),
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-6".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(&event, &template, "ignored compact body");
        let title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();
        let color = payload
            .pointer("/embeds/0/color")
            .and_then(Value::as_u64)
            .unwrap();

        assert_eq!(title, "Contributor updated PR branch");
        assert_eq!(color, parse_embed_color("#0EA5A4").unwrap());
        assert_eq!(
            payload
                .pointer("/embeds")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn discord_payload_limits_commits_and_shortens_ids() {
        let event = UnifiedEvent {
            provider: "github".to_string(),
            event_type: "push".to_string(),
            repository: UnifiedRepository {
                name: "dmxforge".to_string(),
                full_name: "acme/dmxforge".to_string(),
                url: Some("https://github.com/acme/dmxforge".to_string()),
            },
            actor: UnifiedActor {
                name: "Acme".to_string(),
                username: "acme".to_string(),
                url: Some("https://github.com/acme".to_string()),
                avatar_url: Some("https://avatars.githubusercontent.com/u/1?v=4".to_string()),
            },
            branch: Some("main".to_string()),
            compare_url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            commit_count: 6,
            commits: (0..6)
                .map(|index| UnifiedCommit {
                    id: format!("abcdef{index}123456"),
                    short_id: format!("abcdef{index}"),
                    message: format!("Commit {index}"),
                    url: Some(format!(
                        "https://github.com/acme/dmxforge/commit/abcdef{index}123456"
                    )),
                    author_name: Some("Acme".to_string()),
                })
                .collect(),
            title: Some("Push".to_string()),
            description: None,
            status: Some("success".to_string()),
            url: Some("https://github.com/acme/dmxforge/compare/a...b".to_string()),
            timestamp: "2026-03-13T00:00:00Z".to_string(),
            metadata: Value::Null,
        };
        let template = db::MessageTemplateListItem {
            id: "tpl-1".to_string(),
            name: "Compact".to_string(),
            user_id: None,
            format_style: "compact".to_string(),
            body_template: String::new(),
            embed_color: Some("#FF7000".to_string()),
            username_override: Some("DmxForge".to_string()),
            avatar_url_override: None,
            footer_text: Some("Footer".to_string()),
            show_avatar: 1,
            show_repo_link: 1,
            show_branch: 1,
            show_commits: 1,
            show_status_badge: 1,
            show_timestamp: 1,
            is_active: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let payload = build_discord_payload(
            &event,
            &template,
            "Acme pushed 6 commits to acme/dmxforge on main",
        );
        let author_name = payload
            .pointer("/embeds/0/author/name")
            .and_then(Value::as_str)
            .unwrap();
        let author_icon_url = payload
            .pointer("/embeds/0/author/icon_url")
            .and_then(Value::as_str)
            .unwrap();
        let author_url = payload
            .pointer("/embeds/0/author/url")
            .and_then(Value::as_str)
            .unwrap();
        let embed_title = payload
            .pointer("/embeds/0/title")
            .and_then(Value::as_str)
            .unwrap();
        let embed_description = payload.pointer("/embeds/0/description");
        let repository_field = payload
            .pointer("/embeds/0/fields/0/value")
            .and_then(Value::as_str)
            .unwrap();
        let commit_field = payload
            .pointer("/embeds/0/fields/3/value")
            .and_then(Value::as_str)
            .unwrap();

        assert_eq!(author_name, "Acme");
        assert_eq!(
            author_icon_url,
            "https://avatars.githubusercontent.com/u/1?v=4"
        );
        assert_eq!(author_url, "https://github.com/acme");
        assert_eq!(embed_title, "Acme pushed 6 commits");
        assert!(embed_description.is_none());
        assert!(repository_field.contains("[acme/dmxforge]("));
        assert!(commit_field.contains("[abcdef0]("));
        assert!(commit_field.contains("(Acme)"));
        assert!(!commit_field.contains("Commit 5"));
    }
}
