use anyhow::{Context, Result};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use super::{as_sqlite_bool, generate_token};

#[derive(Debug, Clone, FromRow)]
pub struct SourceRecord {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub webhook_secret: Option<String>,
    pub repository_filter: Option<String>,
    pub allowed_branches: Option<String>,
    pub allowed_events: Option<String>,
    pub is_active: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct SourceListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub token: String,
    pub webhook_secret: Option<String>,
    pub repository_filter: Option<String>,
    pub allowed_branches: Option<String>,
    pub allowed_events: Option<String>,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewSource {
    pub user_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub webhook_secret: Option<String>,
    pub repository_filter: Option<String>,
    pub allowed_branches: Option<String>,
    pub allowed_events: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct DestinationListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub webhook_url: String,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewDestination {
    pub user_id: Option<String>,
    pub name: String,
    pub webhook_url: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct MessageTemplateListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub format_style: String,
    pub body_template: String,
    pub embed_color: Option<String>,
    pub username_override: Option<String>,
    pub avatar_url_override: Option<String>,
    pub footer_text: Option<String>,
    pub show_avatar: i64,
    pub show_repo_link: i64,
    pub show_branch: i64,
    pub show_commits: i64,
    pub show_status_badge: i64,
    pub show_timestamp: i64,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewMessageTemplate {
    pub user_id: Option<String>,
    pub name: String,
    pub format_style: String,
    pub body_template: String,
    pub embed_color: Option<String>,
    pub username_override: Option<String>,
    pub avatar_url_override: Option<String>,
    pub footer_text: Option<String>,
    pub show_avatar: bool,
    pub show_repo_link: bool,
    pub show_branch: bool,
    pub show_commits: bool,
    pub show_status_badge: bool,
    pub show_timestamp: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct RoutingRuleListItem {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub destination_id: String,
    pub destination_name: String,
    pub template_id: String,
    pub template_name: String,
    pub provider_filter: Option<String>,
    pub event_type_filter: Option<String>,
    pub branch_prefix_filter: Option<String>,
    pub repository_filter: Option<String>,
    pub skip_keyword: Option<String>,
    pub sort_order: i64,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewRoutingRule {
    pub user_id: Option<String>,
    pub name: String,
    pub source_id: Option<String>,
    pub destination_id: String,
    pub template_id: String,
    pub provider_filter: Option<String>,
    pub event_type_filter: Option<String>,
    pub branch_prefix_filter: Option<String>,
    pub repository_filter: Option<String>,
    pub skip_keyword: Option<String>,
    pub sort_order: i64,
    pub is_active: bool,
}

pub async fn list_sources(pool: &SqlitePool) -> Result<Vec<SourceListItem>> {
    let items = sqlx::query_as::<_, SourceListItem>(
        r#"
        SELECT id, user_id, name, provider, token, webhook_secret, repository_filter, allowed_branches, allowed_events, is_active, created_at, updated_at
        FROM sources
        ORDER BY updated_at DESC, name ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list sources")?;

    Ok(items)
}

pub async fn find_source_by_id(pool: &SqlitePool, id: &str) -> Result<Option<SourceListItem>> {
    let item = sqlx::query_as::<_, SourceListItem>(
        r#"
        SELECT id, user_id, name, provider, token, webhook_secret, repository_filter, allowed_branches, allowed_events, is_active, created_at, updated_at
        FROM sources
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch source by id")?;

    Ok(item)
}

pub async fn find_source_record_by_id(pool: &SqlitePool, id: &str) -> Result<Option<SourceRecord>> {
    let item = sqlx::query_as::<_, SourceRecord>(
        r#"
        SELECT id, user_id, name, provider, webhook_secret, repository_filter, allowed_branches, allowed_events, is_active
        FROM sources
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch source runtime record by id")?;

    Ok(item)
}

pub async fn create_source(pool: &SqlitePool, input: NewSource) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let token = generate_token();

    sqlx::query(
        r#"
        INSERT INTO sources (id, user_id, name, provider, token, webhook_secret, repository_filter, allowed_branches, allowed_events, is_active)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(input.user_id)
    .bind(input.name)
    .bind(input.provider)
    .bind(token)
    .bind(input.webhook_secret)
    .bind(input.repository_filter)
    .bind(input.allowed_branches)
    .bind(input.allowed_events)
    .bind(as_sqlite_bool(input.is_active))
    .execute(pool)
    .await
    .context("failed to create source")?;

    Ok(id)
}

pub async fn update_source(pool: &SqlitePool, id: &str, input: NewSource) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE sources
        SET name = ?, provider = ?, webhook_secret = ?, repository_filter = ?, allowed_branches = ?, allowed_events = ?, is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(input.name)
    .bind(input.provider)
    .bind(input.webhook_secret)
    .bind(input.repository_filter)
    .bind(input.allowed_branches)
    .bind(input.allowed_events)
    .bind(as_sqlite_bool(input.is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to update source")?;

    Ok(())
}

pub async fn set_source_active(pool: &SqlitePool, id: &str, is_active: bool) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE sources
        SET is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(as_sqlite_bool(is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to toggle source status")?;

    Ok(())
}

pub async fn regenerate_source_token(pool: &SqlitePool, id: &str) -> Result<String> {
    let token = generate_token();

    sqlx::query(
        r#"
        UPDATE sources
        SET token = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(&token)
    .bind(id)
    .execute(pool)
    .await
    .context("failed to regenerate source token")?;

    Ok(token)
}

pub async fn delete_source(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sources WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete source")?;

    Ok(())
}

pub async fn list_destinations(pool: &SqlitePool) -> Result<Vec<DestinationListItem>> {
    let items = sqlx::query_as::<_, DestinationListItem>(
        r#"
        SELECT id, user_id, name, webhook_url, is_active, created_at, updated_at
        FROM discord_destinations
        ORDER BY updated_at DESC, name ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list destinations")?;

    Ok(items)
}

pub async fn find_destination_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<DestinationListItem>> {
    let item = sqlx::query_as::<_, DestinationListItem>(
        r#"
        SELECT id, user_id, name, webhook_url, is_active, created_at, updated_at
        FROM discord_destinations
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch destination by id")?;

    Ok(item)
}

pub async fn create_destination(pool: &SqlitePool, input: NewDestination) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO discord_destinations (id, user_id, name, webhook_url, is_active)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(input.user_id)
    .bind(input.name)
    .bind(input.webhook_url)
    .bind(as_sqlite_bool(input.is_active))
    .execute(pool)
    .await
    .context("failed to create destination")?;

    Ok(id)
}

pub async fn update_destination(pool: &SqlitePool, id: &str, input: NewDestination) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE discord_destinations
        SET name = ?, webhook_url = ?, is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(input.name)
    .bind(input.webhook_url)
    .bind(as_sqlite_bool(input.is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to update destination")?;

    Ok(())
}

pub async fn set_destination_active(pool: &SqlitePool, id: &str, is_active: bool) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE discord_destinations
        SET is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(as_sqlite_bool(is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to toggle destination status")?;

    Ok(())
}

pub async fn delete_destination(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM discord_destinations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete destination")?;

    Ok(())
}

pub async fn list_message_templates(pool: &SqlitePool) -> Result<Vec<MessageTemplateListItem>> {
    let items = sqlx::query_as::<_, MessageTemplateListItem>(
        r#"
        SELECT id, user_id, name, format_style, body_template, embed_color, username_override, avatar_url_override, footer_text, show_avatar, show_repo_link, show_branch, show_commits, show_status_badge, show_timestamp, is_active, created_at, updated_at
        FROM message_templates
        ORDER BY updated_at DESC, name ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list message templates")?;

    Ok(items)
}

pub async fn find_message_template_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<MessageTemplateListItem>> {
    let item = sqlx::query_as::<_, MessageTemplateListItem>(
        r#"
        SELECT id, user_id, name, format_style, body_template, embed_color, username_override, avatar_url_override, footer_text, show_avatar, show_repo_link, show_branch, show_commits, show_status_badge, show_timestamp, is_active, created_at, updated_at
        FROM message_templates
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch message template by id")?;

    Ok(item)
}

pub async fn create_message_template(
    pool: &SqlitePool,
    input: NewMessageTemplate,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO message_templates (id, user_id, name, format_style, body_template, embed_color, username_override, avatar_url_override, footer_text, show_avatar, show_repo_link, show_branch, show_commits, show_status_badge, show_timestamp, is_active)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(input.user_id)
    .bind(input.name)
    .bind(input.format_style)
    .bind(input.body_template)
    .bind(input.embed_color)
    .bind(input.username_override)
    .bind(input.avatar_url_override)
    .bind(input.footer_text)
    .bind(as_sqlite_bool(input.show_avatar))
    .bind(as_sqlite_bool(input.show_repo_link))
    .bind(as_sqlite_bool(input.show_branch))
    .bind(as_sqlite_bool(input.show_commits))
    .bind(as_sqlite_bool(input.show_status_badge))
    .bind(as_sqlite_bool(input.show_timestamp))
    .bind(as_sqlite_bool(input.is_active))
    .execute(pool)
    .await
    .context("failed to create message template")?;

    Ok(id)
}

pub async fn update_message_template(
    pool: &SqlitePool,
    id: &str,
    input: NewMessageTemplate,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE message_templates
        SET name = ?, format_style = ?, body_template = ?, embed_color = ?, username_override = ?, avatar_url_override = ?, footer_text = ?, show_avatar = ?, show_repo_link = ?, show_branch = ?, show_commits = ?, show_status_badge = ?, show_timestamp = ?, is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(input.name)
    .bind(input.format_style)
    .bind(input.body_template)
    .bind(input.embed_color)
    .bind(input.username_override)
    .bind(input.avatar_url_override)
    .bind(input.footer_text)
    .bind(as_sqlite_bool(input.show_avatar))
    .bind(as_sqlite_bool(input.show_repo_link))
    .bind(as_sqlite_bool(input.show_branch))
    .bind(as_sqlite_bool(input.show_commits))
    .bind(as_sqlite_bool(input.show_status_badge))
    .bind(as_sqlite_bool(input.show_timestamp))
    .bind(as_sqlite_bool(input.is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to update message template")?;

    Ok(())
}

pub async fn set_message_template_active(
    pool: &SqlitePool,
    id: &str,
    is_active: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE message_templates
        SET is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(as_sqlite_bool(is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to toggle template status")?;

    Ok(())
}

pub async fn delete_message_template(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM message_templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete message template")?;

    Ok(())
}

pub async fn list_routing_rules(pool: &SqlitePool) -> Result<Vec<RoutingRuleListItem>> {
    let items = sqlx::query_as::<_, RoutingRuleListItem>(
        r#"
        SELECT rr.id, rr.user_id, rr.name, rr.source_id, s.name AS source_name, rr.destination_id, d.name AS destination_name, rr.template_id, t.name AS template_name, rr.provider_filter, rr.event_type_filter, rr.branch_prefix_filter, rr.repository_filter, rr.skip_keyword, rr.sort_order, rr.is_active, rr.created_at, rr.updated_at
        FROM routing_rules rr
        LEFT JOIN sources s ON s.id = rr.source_id
        INNER JOIN discord_destinations d ON d.id = rr.destination_id
        INNER JOIN message_templates t ON t.id = rr.template_id
        ORDER BY rr.sort_order ASC, rr.updated_at DESC, rr.name ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list routing rules")?;

    Ok(items)
}

pub async fn find_routing_rule_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<RoutingRuleListItem>> {
    let item = sqlx::query_as::<_, RoutingRuleListItem>(
        r#"
        SELECT rr.id, rr.user_id, rr.name, rr.source_id, s.name AS source_name, rr.destination_id, d.name AS destination_name, rr.template_id, t.name AS template_name, rr.provider_filter, rr.event_type_filter, rr.branch_prefix_filter, rr.repository_filter, rr.skip_keyword, rr.sort_order, rr.is_active, rr.created_at, rr.updated_at
        FROM routing_rules rr
        LEFT JOIN sources s ON s.id = rr.source_id
        INNER JOIN discord_destinations d ON d.id = rr.destination_id
        INNER JOIN message_templates t ON t.id = rr.template_id
        WHERE rr.id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch routing rule by id")?;

    Ok(item)
}

pub async fn create_routing_rule(pool: &SqlitePool, input: NewRoutingRule) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO routing_rules (id, user_id, source_id, destination_id, template_id, name, provider_filter, event_type_filter, branch_prefix_filter, repository_filter, skip_keyword, sort_order, is_active)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(input.user_id)
    .bind(input.source_id)
    .bind(input.destination_id)
    .bind(input.template_id)
    .bind(input.name)
    .bind(input.provider_filter)
    .bind(input.event_type_filter)
    .bind(input.branch_prefix_filter)
    .bind(input.repository_filter)
    .bind(input.skip_keyword)
    .bind(input.sort_order)
    .bind(as_sqlite_bool(input.is_active))
    .execute(pool)
    .await
    .context("failed to create routing rule")?;

    Ok(id)
}

pub async fn update_routing_rule(pool: &SqlitePool, id: &str, input: NewRoutingRule) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE routing_rules
        SET source_id = ?, destination_id = ?, template_id = ?, name = ?, provider_filter = ?, event_type_filter = ?, branch_prefix_filter = ?, repository_filter = ?, skip_keyword = ?, sort_order = ?, is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(input.source_id)
    .bind(input.destination_id)
    .bind(input.template_id)
    .bind(input.name)
    .bind(input.provider_filter)
    .bind(input.event_type_filter)
    .bind(input.branch_prefix_filter)
    .bind(input.repository_filter)
    .bind(input.skip_keyword)
    .bind(input.sort_order)
    .bind(as_sqlite_bool(input.is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to update routing rule")?;

    Ok(())
}

pub async fn set_routing_rule_active(pool: &SqlitePool, id: &str, is_active: bool) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE routing_rules
        SET is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(as_sqlite_bool(is_active))
    .bind(id)
    .execute(pool)
    .await
    .context("failed to toggle routing rule status")?;

    Ok(())
}

pub async fn delete_routing_rule(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM routing_rules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete routing rule")?;

    Ok(())
}

pub async fn find_source_by_provider_token(
    pool: &SqlitePool,
    provider: &str,
    token: &str,
) -> Result<Option<SourceRecord>> {
    let source = sqlx::query_as::<_, SourceRecord>(
        r#"
        SELECT id, user_id, name, provider, webhook_secret, repository_filter, allowed_branches, allowed_events, is_active
        FROM sources
        WHERE provider = ? AND token = ?
        LIMIT 1
        "#,
    )
    .bind(provider)
    .bind(token)
    .fetch_optional(pool)
    .await
    .context("failed to query source by provider/token")?;

    Ok(source)
}

pub async fn seed_default_templates(pool: &SqlitePool) -> Result<()> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM message_templates")
        .fetch_one(pool)
        .await
        .context("failed to count message templates")?;

    if count > 0 {
        return Ok(());
    }

    let defaults = [
        NewMessageTemplate {
            user_id: None,
            name: "Compact Push".to_string(),
            format_style: "compact".to_string(),
            body_template: "{{ actor.name }} pushed {{ commit_count }} commits to {{ repository.full_name }} on {{ branch }}".to_string(),
            embed_color: Some("#3B82F6".to_string()),
            username_override: None,
            avatar_url_override: None,
            footer_text: None,
            show_avatar: true,
            show_repo_link: true,
            show_branch: true,
            show_commits: true,
            show_status_badge: true,
            show_timestamp: true,
            is_active: true,
        },
        NewMessageTemplate {
            user_id: None,
            name: "Detailed Activity".to_string(),
            format_style: "detailed".to_string(),
            body_template: "{{ actor.name }} triggered {{ event_type }} on {{ repository.full_name }}\n{% for commit in commits -%}\n- {{ commit.id }} {{ commit.message }}\n{% endfor -%}".to_string(),
            embed_color: Some("#10B981".to_string()),
            username_override: None,
            avatar_url_override: None,
            footer_text: None,
            show_avatar: true,
            show_repo_link: true,
            show_branch: true,
            show_commits: true,
            show_status_badge: true,
            show_timestamp: true,
            is_active: true,
        },
        NewMessageTemplate {
            user_id: None,
            name: "Release Notes".to_string(),
            format_style: "release".to_string(),
            body_template: "Release for {{ repository.full_name }} on {{ branch }}\nCompare: {{ compare_url }}".to_string(),
            embed_color: Some("#F59E0B".to_string()),
            username_override: Some("Release Bot".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: true,
            show_repo_link: true,
            show_branch: true,
            show_commits: false,
            show_status_badge: true,
            show_timestamp: true,
            is_active: true,
        },
        NewMessageTemplate {
            user_id: None,
            name: "CI Alert".to_string(),
            format_style: "alert".to_string(),
            body_template: "Pipeline alert for {{ repository.full_name }} by {{ actor.name }} on {{ branch }}".to_string(),
            embed_color: Some("#EF4444".to_string()),
            username_override: Some("CI Alert".to_string()),
            avatar_url_override: None,
            footer_text: None,
            show_avatar: true,
            show_repo_link: true,
            show_branch: true,
            show_commits: false,
            show_status_badge: true,
            show_timestamp: true,
            is_active: true,
        },
    ];

    for template in defaults {
        create_message_template(pool, template).await?;
    }

    Ok(())
}

pub async fn assign_unowned_resources_to_user(pool: &SqlitePool, user_id: &str) -> Result<()> {
    for table in [
        "sources",
        "discord_destinations",
        "message_templates",
        "routing_rules",
    ] {
        let statement = format!("UPDATE {table} SET user_id = ? WHERE user_id IS NULL");
        sqlx::query(&statement)
            .bind(user_id)
            .execute(pool)
            .await
            .with_context(|| format!("failed to assign unowned rows for {table}"))?;
    }

    Ok(())
}
