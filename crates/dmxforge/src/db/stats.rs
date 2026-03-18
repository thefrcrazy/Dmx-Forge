use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct InstanceCounts {
    pub user_count: i64,
    pub active_user_count: i64,
    pub active_session_count: i64,
    pub source_count: i64,
    pub destination_count: i64,
    pub template_count: i64,
    pub rule_count: i64,
    pub delivery_count: i64,
    pub audit_log_count: i64,
}

#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub total_deliveries: i64,
    pub processed_deliveries: i64,
    pub failed_deliveries: i64,
    pub discord_messages_sent: i64,
    pub activity: Vec<ActivityPoint>,
    pub top_repositories: Vec<TopMetric>,
    pub top_events: Vec<TopMetric>,
    pub recent_deliveries: Vec<RecentDelivery>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityPoint {
    pub day_label: String,
    pub count: i64,
    pub height: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct TopMetric {
    pub label: String,
    pub value: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct RecentDelivery {
    pub short_id: String,
    pub provider: String,
    pub event_type: String,
    pub repository: String,
    pub status: String,
    pub status_class: String,
    pub received_at: String,
}

pub async fn fetch_instance_counts(pool: &SqlitePool) -> Result<InstanceCounts> {
    let row = sqlx::query(
        r#"
        SELECT
            (SELECT COUNT(*) FROM users) as user_count,
            (SELECT COUNT(*) FROM users WHERE is_active = 1) as active_user_count,
            (SELECT COUNT(*) FROM sessions WHERE datetime(expires_at) > datetime('now')) as active_session_count,
            (SELECT COUNT(*) FROM sources) as source_count,
            (SELECT COUNT(*) FROM discord_destinations) as destination_count,
            (SELECT COUNT(*) FROM message_templates) as template_count,
            (SELECT COUNT(*) FROM routing_rules) as rule_count,
            (SELECT COUNT(*) FROM webhook_deliveries) as delivery_count,
            (SELECT COUNT(*) FROM audit_logs) as audit_log_count
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to fetch instance counts")?;

    Ok(InstanceCounts {
        user_count: row.get(0),
        active_user_count: row.get(1),
        active_session_count: row.get(2),
        source_count: row.get(3),
        destination_count: row.get(4),
        template_count: row.get(5),
        rule_count: row.get(6),
        delivery_count: row.get(7),
        audit_log_count: row.get(8),
    })
}

pub async fn fetch_dashboard_snapshot(
    pool: &SqlitePool,
    visible_source_ids: Option<&[String]>,
) -> Result<DashboardSnapshot> {
    let total_deliveries =
        count_deliveries(pool, visible_source_ids, None).await?;
    let processed_deliveries =
        count_deliveries(pool, visible_source_ids, Some("processed")).await?;
    let failed_deliveries = count_deliveries(pool, visible_source_ids, Some("failed")).await?;
    let discord_messages_sent =
        count_sent_discord_messages(pool, visible_source_ids).await?;

    let mut activity_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT substr(received_at, 1, 10) AS day, COUNT(*) AS value
        FROM webhook_deliveries
        WHERE date(received_at) >= date('now', '-6 day')
        "#,
    );
    apply_source_scope(&mut activity_builder, "source_id", visible_source_ids);
    activity_builder.push(
        r#"
        GROUP BY substr(received_at, 1, 10)
        ORDER BY day ASC
        "#,
    );
    let activity_rows = activity_builder
        .build()
        .fetch_all(pool)
        .await
        .context("failed to load 7-day activity")?;

    let mut activity_map = HashMap::new();
    for row in activity_rows {
        activity_map.insert(row.get::<String, _>("day"), row.get::<i64, _>("value"));
    }

    let mut activity = Vec::with_capacity(7);
    let today = Utc::now().date_naive();

    for days_ago in (0..7).rev() {
        let day = today - Duration::days(days_ago);
        let key = day.format("%Y-%m-%d").to_string();
        let count = *activity_map.get(&key).unwrap_or(&0);
        activity.push(ActivityPoint {
            day_label: format_day(day),
            count,
            height: 24,
        });
    }

    let max_count = activity.iter().map(|point| point.count).max().unwrap_or(0);
    for point in &mut activity {
        point.height = if point.count == 0 || max_count == 0 {
            18
        } else {
            18 + ((point.count as f64 / max_count as f64) * 92.0).round() as i64
        };
    }

    let mut top_repositories_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT repository AS label, COUNT(*) AS value
        FROM webhook_deliveries
        WHERE repository IS NOT NULL AND repository <> ''
        "#,
    );
    apply_source_scope(&mut top_repositories_builder, "source_id", visible_source_ids);
    top_repositories_builder.push(
        r#"
        GROUP BY repository
        ORDER BY value DESC, repository ASC
        LIMIT 5
        "#,
    );
    let top_repositories = top_repositories_builder
        .build_query_as::<TopMetric>()
        .fetch_all(pool)
        .await
        .context("failed to load top repositories")?;

    let mut top_events_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT event_type AS label, COUNT(*) AS value
        FROM webhook_deliveries
        WHERE event_type IS NOT NULL AND event_type <> ''
        "#,
    );
    apply_source_scope(&mut top_events_builder, "source_id", visible_source_ids);
    top_events_builder.push(
        r#"
        GROUP BY event_type
        ORDER BY value DESC, event_type ASC
        LIMIT 5
        "#,
    );
    let top_events = top_events_builder
        .build_query_as::<TopMetric>()
        .fetch_all(pool)
        .await
        .context("failed to load top event types")?;

    let mut recent_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            substr(id, 1, 8) AS short_id,
            provider,
            COALESCE(event_type, 'unknown') AS event_type,
            COALESCE(repository, 'n/a') AS repository,
            status,
            CASE status
                WHEN 'processed' THEN 'success'
                WHEN 'failed' THEN 'danger'
                WHEN 'skipped' THEN 'warning'
                WHEN 'pending' THEN 'pending'
                ELSE 'info'
            END AS status_class,
            received_at
        FROM webhook_deliveries
        "#,
    );
    apply_source_scope(&mut recent_builder, "source_id", visible_source_ids);
    recent_builder.push(
        r#"
        ORDER BY received_at DESC
        LIMIT 10
        "#,
    );
    let recent_deliveries = recent_builder
        .build_query_as::<RecentDelivery>()
        .fetch_all(pool)
        .await
        .context("failed to load recent deliveries")?;

    Ok(DashboardSnapshot {
        total_deliveries,
        processed_deliveries,
        failed_deliveries,
        discord_messages_sent,
        activity,
        top_repositories,
        top_events,
        recent_deliveries,
    })
}

fn format_day(day: NaiveDate) -> String {
    day.format("%d/%m").to_string()
}

async fn count_deliveries(
    pool: &SqlitePool,
    visible_source_ids: Option<&[String]>,
    status: Option<&str>,
) -> Result<i64> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM webhook_deliveries WHERE 1 = 1");
    apply_source_scope(&mut builder, "source_id", visible_source_ids);
    if let Some(status) = status {
        builder.push(" AND status = ");
        builder.push_bind(status);
    }

    builder
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .context("failed to count deliveries")
}

async fn count_sent_discord_messages(
    pool: &SqlitePool,
    visible_source_ids: Option<&[String]>,
) -> Result<i64> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT COUNT(*)
        FROM discord_messages dm
        INNER JOIN webhook_deliveries wd ON wd.id = dm.delivery_id
        WHERE dm.status = 'sent'
        "#,
    );
    apply_source_scope(&mut builder, "wd.source_id", visible_source_ids);

    builder
        .build_query_scalar::<i64>()
        .fetch_one(pool)
        .await
        .context("failed to count sent discord messages")
}

fn apply_source_scope<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    column: &str,
    visible_source_ids: Option<&'a [String]>,
) {
    if let Some(source_ids) = visible_source_ids {
        if source_ids.is_empty() {
            builder.push(" AND 1 = 0");
        } else {
            builder.push(format!(" AND {column} IN ("));
            let mut separated = builder.separated(", ");
            for source_id in source_ids {
                separated.push_bind(source_id);
            }
            separated.push_unseparated(")");
        }
    }
}
