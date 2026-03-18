use anyhow::{Context, Result};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NewWebhookDelivery {
    pub source_id: String,
    pub provider: String,
    pub event_type: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub raw_headers: String,
    pub raw_payload: String,
}

#[derive(Debug, Clone)]
pub struct NewDiscordMessageAttempt {
    pub delivery_id: String,
    pub destination_id: Option<String>,
    pub request_payload: String,
    pub response_status: Option<i64>,
    pub response_body: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct DeliveryListFilters {
    pub status: Option<String>,
    pub source_id: Option<String>,
    pub provider: Option<String>,
    pub event_type: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeliveryListPage {
    pub items: Vec<DeliveryListItem>,
    pub total_count: i64,
    pub page: usize,
    pub per_page: usize,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeliveryListItem {
    pub id: String,
    pub short_id: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub provider: String,
    pub event_type: String,
    pub repository: String,
    pub branch: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub received_at: String,
    pub processed_at: Option<String>,
    pub sent_count: i64,
    pub failed_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeliveryDetail {
    pub id: String,
    pub source_id: Option<String>,
    pub source_name: Option<String>,
    pub provider: String,
    pub event_type: String,
    pub repository: String,
    pub branch: Option<String>,
    pub status: String,
    pub failure_reason: Option<String>,
    pub raw_headers: String,
    pub raw_payload: String,
    pub normalized_event: Option<String>,
    pub received_at: String,
    pub processed_at: Option<String>,
    pub sent_count: i64,
    pub failed_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeliveryMessageAttempt {
    pub id: String,
    pub destination_id: Option<String>,
    pub destination_name: Option<String>,
    pub request_payload: String,
    pub response_status: Option<i64>,
    pub response_body: Option<String>,
    pub status: String,
    pub attempted_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeliveryReplayRecord {
    pub source_id: Option<String>,
    pub provider: String,
    pub event_type: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub raw_headers: String,
    pub raw_payload: String,
}

pub async fn save_incoming_delivery(
    pool: &SqlitePool,
    delivery: NewWebhookDelivery,
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO webhook_deliveries (
            id, source_id, provider, event_type, repository, branch, status, raw_headers, raw_payload
        )
        VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
        "#,
    )
    .bind(id.to_string())
    .bind(delivery.source_id)
    .bind(delivery.provider)
    .bind(delivery.event_type)
    .bind(delivery.repository)
    .bind(delivery.branch)
    .bind(delivery.raw_headers)
    .bind(delivery.raw_payload)
    .execute(pool)
    .await
    .context("failed to persist incoming delivery")?;

    Ok(id)
}

pub async fn mark_delivery_skipped(pool: &SqlitePool, id: Uuid, reason: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'skipped', failure_reason = ?, processed_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(reason)
    .bind(id.to_string())
    .execute(pool)
    .await
    .context("failed to mark delivery as skipped")?;

    Ok(())
}

pub async fn mark_delivery_processed(
    pool: &SqlitePool,
    id: Uuid,
    normalized_event: &str,
    failure_reason: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'processed', failure_reason = ?, normalized_event = ?, processed_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(failure_reason)
    .bind(normalized_event)
    .bind(id.to_string())
    .execute(pool)
    .await
    .context("failed to mark delivery as processed")?;

    Ok(())
}

pub async fn mark_delivery_failed(
    pool: &SqlitePool,
    id: Uuid,
    reason: &str,
    normalized_event: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'failed', failure_reason = ?, normalized_event = ?, processed_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(reason)
    .bind(normalized_event)
    .bind(id.to_string())
    .execute(pool)
    .await
    .context("failed to mark delivery as failed")?;

    Ok(())
}

pub async fn mark_delivery_skipped_with_event(
    pool: &SqlitePool,
    id: Uuid,
    reason: &str,
    normalized_event: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'skipped', failure_reason = ?, normalized_event = ?, processed_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(reason)
    .bind(normalized_event)
    .bind(id.to_string())
    .execute(pool)
    .await
    .context("failed to mark delivery as skipped with normalized event")?;

    Ok(())
}

pub async fn save_discord_message_attempt(
    pool: &SqlitePool,
    attempt: NewDiscordMessageAttempt,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO discord_messages (
            id, delivery_id, destination_id, request_payload, response_status, response_body, status
        )
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(attempt.delivery_id)
    .bind(attempt.destination_id)
    .bind(attempt.request_payload)
    .bind(attempt.response_status)
    .bind(attempt.response_body)
    .bind(attempt.status)
    .execute(pool)
    .await
    .context("failed to persist Discord message attempt")?;

    Ok(id)
}

pub async fn list_delivery_summaries(
    pool: &SqlitePool,
    filters: &DeliveryListFilters,
    page: usize,
    per_page: usize,
    visible_source_ids: Option<&[String]>,
) -> Result<DeliveryListPage> {
    let current_page = page.max(1);
    let offset = (current_page - 1) * per_page;

    let mut count_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT COUNT(*)
        FROM webhook_deliveries wd
        LEFT JOIN sources s ON s.id = wd.source_id
        WHERE 1 = 1
        "#,
    );
    apply_source_scope(&mut count_builder, visible_source_ids);
    apply_delivery_filters(&mut count_builder, filters);

    let total_count = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .context("failed to count delivery summaries")?;

    let mut list_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            wd.id,
            substr(wd.id, 1, 8) AS short_id,
            wd.source_id,
            s.name AS source_name,
            wd.provider,
            COALESCE(wd.event_type, 'unknown') AS event_type,
            COALESCE(wd.repository, 'n/a') AS repository,
            COALESCE(wd.branch, 'n/a') AS branch,
            wd.status,
            wd.failure_reason,
            wd.received_at,
            wd.processed_at,
            COALESCE(SUM(CASE WHEN dm.status = 'sent' THEN 1 ELSE 0 END), 0) AS sent_count,
            COALESCE(SUM(CASE WHEN dm.status = 'failed' THEN 1 ELSE 0 END), 0) AS failed_count
        FROM webhook_deliveries wd
        LEFT JOIN sources s ON s.id = wd.source_id
        LEFT JOIN discord_messages dm ON dm.delivery_id = wd.id
        WHERE 1 = 1
        "#,
    );
    apply_source_scope(&mut list_builder, visible_source_ids);
    apply_delivery_filters(&mut list_builder, filters);
    list_builder.push(
        r#"
        GROUP BY wd.id, wd.source_id, s.name, wd.provider, wd.event_type, wd.repository, wd.branch, wd.status, wd.failure_reason, wd.received_at, wd.processed_at
        ORDER BY wd.received_at DESC
        "#,
    );
    list_builder.push(" LIMIT ");
    list_builder.push_bind(per_page as i64);
    list_builder.push(" OFFSET ");
    list_builder.push_bind(offset as i64);

    let items = list_builder
        .build_query_as::<DeliveryListItem>()
        .fetch_all(pool)
        .await
        .context("failed to list delivery summaries")?;

    Ok(DeliveryListPage {
        items,
        total_count,
        page: current_page,
        per_page,
    })
}

pub async fn find_delivery_detail(
    pool: &SqlitePool,
    id: &str,
    visible_source_ids: Option<&[String]>,
) -> Result<Option<DeliveryDetail>> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            wd.id,
            wd.source_id,
            s.name AS source_name,
            wd.provider,
            COALESCE(wd.event_type, 'unknown') AS event_type,
            COALESCE(wd.repository, 'n/a') AS repository,
            wd.branch,
            wd.status,
            wd.failure_reason,
            wd.raw_headers,
            wd.raw_payload,
            wd.normalized_event,
            wd.received_at,
            wd.processed_at,
            COALESCE(SUM(CASE WHEN dm.status = 'sent' THEN 1 ELSE 0 END), 0) AS sent_count,
            COALESCE(SUM(CASE WHEN dm.status = 'failed' THEN 1 ELSE 0 END), 0) AS failed_count
        FROM webhook_deliveries wd
        LEFT JOIN sources s ON s.id = wd.source_id
        LEFT JOIN discord_messages dm ON dm.delivery_id = wd.id
        WHERE wd.id = 
        "#,
    );
    builder.push_bind(id);
    apply_source_scope(&mut builder, visible_source_ids);
    builder.push(
        r#"
        GROUP BY wd.id, wd.source_id, s.name, wd.provider, wd.event_type, wd.repository, wd.branch, wd.status, wd.failure_reason, wd.raw_headers, wd.raw_payload, wd.normalized_event, wd.received_at, wd.processed_at
        LIMIT 1
        "#,
    );

    let item = builder
        .build_query_as::<DeliveryDetail>()
        .fetch_optional(pool)
        .await
        .context("failed to load delivery detail")?;

    Ok(item)
}

pub async fn list_delivery_message_attempts(
    pool: &SqlitePool,
    delivery_id: &str,
) -> Result<Vec<DeliveryMessageAttempt>> {
    let items = sqlx::query_as::<_, DeliveryMessageAttempt>(
        r#"
        SELECT
            dm.id,
            dm.destination_id,
            dd.name AS destination_name,
            dm.request_payload,
            dm.response_status,
            dm.response_body,
            dm.status,
            dm.attempted_at
        FROM discord_messages dm
        LEFT JOIN discord_destinations dd ON dd.id = dm.destination_id
        WHERE dm.delivery_id = ?
        ORDER BY dm.attempted_at ASC
        "#,
    )
    .bind(delivery_id)
    .fetch_all(pool)
    .await
    .context("failed to list Discord message attempts")?;

    Ok(items)
}

pub async fn find_delivery_replay_record(
    pool: &SqlitePool,
    delivery_id: &str,
    visible_source_ids: Option<&[String]>,
) -> Result<Option<DeliveryReplayRecord>> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT source_id, provider, event_type, repository, branch, raw_headers, raw_payload
        FROM webhook_deliveries
        WHERE id =
        "#,
    );
    builder.push_bind(delivery_id);
    if let Some(source_ids) = visible_source_ids {
        if source_ids.is_empty() {
            builder.push(" AND 1 = 0");
        } else {
            builder.push(" AND source_id IN (");
            let mut separated = builder.separated(", ");
            for source_id in source_ids {
                separated.push_bind(source_id);
            }
            separated.push_unseparated(")");
        }
    }
    builder.push(
        r#"
        LIMIT 1
        "#,
    );

    let item = builder
        .build_query_as::<DeliveryReplayRecord>()
        .fetch_optional(pool)
        .await
        .context("failed to load replay record for delivery")?;

    Ok(item)
}

fn apply_delivery_filters<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    filters: &DeliveryListFilters,
) {
    if let Some(status) = filters
        .status
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND wd.status = ");
        builder.push_bind(status.trim().to_string());
    }

    if let Some(source_id) = filters
        .source_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND wd.source_id = ");
        builder.push_bind(source_id.trim().to_string());
    }

    if let Some(provider) = filters
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND wd.provider = ");
        builder.push_bind(provider.trim().to_string());
    }

    if let Some(event_type) = filters
        .event_type
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND COALESCE(wd.event_type, '') = ");
        builder.push_bind(event_type.trim().to_string());
    }

    if let Some(date_from) = filters
        .date_from
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND date(wd.received_at) >= date(");
        builder.push_bind(date_from.trim().to_string());
        builder.push(")");
    }

    if let Some(date_to) = filters
        .date_to
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder.push(" AND date(wd.received_at) <= date(");
        builder.push_bind(date_to.trim().to_string());
        builder.push(")");
    }

    if let Some(search) = filters
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let like = format!("%{search}%");
        builder.push(" AND (COALESCE(wd.repository, '') LIKE ");
        builder.push_bind(like.clone());
        builder.push(" OR wd.raw_payload LIKE ");
        builder.push_bind(like.clone());
        builder.push(" OR wd.id LIKE ");
        builder.push_bind(like);
        builder.push(")");
    }
}

fn apply_source_scope<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    visible_source_ids: Option<&'a [String]>,
) {
    if let Some(source_ids) = visible_source_ids {
        if source_ids.is_empty() {
            builder.push(" AND 1 = 0");
        } else {
            builder.push(" AND wd.source_id IN (");
            let mut separated = builder.separated(", ");
            for source_id in source_ids {
                separated.push_bind(source_id);
            }
            separated.push_unseparated(")");
        }
    }
}
