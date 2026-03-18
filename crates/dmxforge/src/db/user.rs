use anyhow::{Context, Result};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use super::as_sqlite_bool;

#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub is_active: bool,
    pub parent_user_id: Option<String>,
    pub created_by_user_id: Option<String>,
    pub permissions_json: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AuthUserRecord {
    pub id: String,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub is_active: i64,
    pub permissions_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: String,
    pub user_id: String,
    pub csrf_token: String,
    pub expires_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionUserRecord {
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub is_active: i64,
    pub permissions_json: Option<String>,
    pub csrf_token: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct UserListItem {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub is_active: i64,
    pub permissions_json: Option<String>,
    pub parent_user_id: Option<String>,
    pub parent_username: Option<String>,
    pub created_by_user_id: Option<String>,
    pub creator_username: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub child_count: i64,
    pub active_session_count: i64,
    pub last_seen_at: Option<String>,
    pub source_count: i64,
    pub destination_count: i64,
    pub template_count: i64,
    pub rule_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct ActiveSessionListItem {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub permissions_json: Option<String>,
    pub created_by_user_id: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub is_active: i64,
    pub parent_user_id: Option<String>,
    pub created_by_user_id: Option<String>,
    pub permissions_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn admin_user_exists(pool: &SqlitePool) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE role IN ('superadmin', 'admin')",
    )
    .fetch_one(pool)
    .await
    .context("failed to count admin users")?;

    Ok(count > 0)
}

pub async fn create_user(pool: &SqlitePool, input: NewUser) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        r#"
        INSERT INTO users (
            id,
            username,
            email,
            password_hash,
            role,
            is_active,
            parent_user_id,
            created_by_user_id,
            permissions_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(input.username)
    .bind(input.email)
    .bind(input.password_hash)
    .bind(input.role)
    .bind(as_sqlite_bool(input.is_active))
    .bind(input.parent_user_id)
    .bind(input.created_by_user_id)
    .bind(input.permissions_json)
    .execute(pool)
    .await
    .context("failed to create user")?;

    Ok(id)
}

pub async fn find_user_by_login(pool: &SqlitePool, login: &str) -> Result<Option<AuthUserRecord>> {
    let item = sqlx::query_as::<_, AuthUserRecord>(
        r#"
        SELECT id, username, email, password_hash, role, is_active, permissions_json
        FROM users
        WHERE lower(email) = lower(?) OR lower(username) = lower(?)
        LIMIT 1
        "#,
    )
    .bind(login)
    .bind(login)
    .fetch_optional(pool)
    .await
    .context("failed to find auth user by login")?;

    Ok(item)
}

pub async fn create_session(pool: &SqlitePool, input: NewSession) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO sessions (
            id,
            user_id,
            csrf_token,
            expires_at,
            ip_address,
            user_agent
        )
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(input.id)
    .bind(input.user_id)
    .bind(input.csrf_token)
    .bind(input.expires_at)
    .bind(input.ip_address)
    .bind(input.user_agent)
    .execute(pool)
    .await
    .context("failed to create auth session")?;

    Ok(())
}

pub async fn find_session_user(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Option<SessionUserRecord>> {
    let item = sqlx::query_as::<_, SessionUserRecord>(
        r#"
        SELECT
            u.id AS user_id,
            u.username,
            u.email,
            u.role,
            u.is_active,
            u.permissions_json,
            s.csrf_token,
            s.last_seen_at
        FROM sessions s
        INNER JOIN users u ON u.id = s.user_id
        WHERE s.id = ? AND datetime(s.expires_at) > datetime('now')
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .context("failed to find auth session user")?;

    Ok(item)
}

pub async fn find_user_by_id(pool: &SqlitePool, user_id: &str) -> Result<Option<UserRecord>> {
    let item = sqlx::query_as::<_, UserRecord>(
        r#"
        SELECT
            id,
            username,
            email,
            password_hash,
            role,
            is_active,
            parent_user_id,
            created_by_user_id,
            permissions_json,
            created_at,
            updated_at
        FROM users
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("failed to fetch user by id")?;

    Ok(item)
}

pub async fn update_user(
    pool: &SqlitePool,
    user_id: &str,
    username: &str,
    email: &str,
    role: &str,
    is_active: bool,
    parent_user_id: Option<String>,
    created_by_user_id: Option<String>,
    permissions_json: Option<String>,
    password_hash: Option<String>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE users
        SET
            username = ?,
            email = ?,
            role = ?,
            is_active = ?,
            parent_user_id = ?,
            created_by_user_id = ?,
            permissions_json = ?,
            password_hash = COALESCE(?, password_hash),
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(username)
    .bind(email)
    .bind(role)
    .bind(as_sqlite_bool(is_active))
    .bind(parent_user_id)
    .bind(created_by_user_id)
    .bind(permissions_json)
    .bind(password_hash)
    .bind(user_id)
    .execute(pool)
    .await
    .context("failed to update user")?;

    Ok(())
}

pub async fn set_user_active(pool: &SqlitePool, user_id: &str, is_active: bool) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE users
        SET is_active = ?, updated_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(as_sqlite_bool(is_active))
    .bind(user_id)
    .execute(pool)
    .await
    .context("failed to update user status")?;

    Ok(())
}

pub async fn delete_user(pool: &SqlitePool, user_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(pool)
        .await
        .context("failed to delete user")?;

    Ok(())
}

pub async fn count_users_with_role(pool: &SqlitePool, role: &str) -> Result<i64> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role = ?")
        .bind(role)
        .fetch_one(pool)
        .await
        .context("failed to count users with role")?;

    Ok(count)
}

pub async fn list_descendant_user_ids(pool: &SqlitePool, root_user_id: &str) -> Result<Vec<String>> {
    let items = sqlx::query_scalar::<_, String>(
        r#"
        WITH RECURSIVE descendants(id) AS (
            SELECT id
            FROM users
            WHERE parent_user_id = ?
            UNION ALL
            SELECT u.id
            FROM users u
            INNER JOIN descendants d ON d.id = u.parent_user_id
        )
        SELECT id FROM descendants
        "#,
    )
    .bind(root_user_id)
    .fetch_all(pool)
    .await
    .context("failed to list descendant user ids")?;

    Ok(items)
}

pub async fn touch_session(pool: &SqlitePool, session_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE sessions
        SET last_seen_at = CURRENT_TIMESTAMP
        WHERE id = ?
        "#,
    )
    .bind(session_id)
    .execute(pool)
    .await
    .context("failed to touch auth session")?;

    Ok(())
}

pub async fn delete_session(pool: &SqlitePool, session_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(session_id)
        .execute(pool)
        .await
        .context("failed to delete auth session")?;

    Ok(())
}

pub async fn delete_expired_sessions(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE datetime(expires_at) <= datetime('now')")
        .execute(pool)
        .await
        .context("failed to delete expired auth sessions")?;

    Ok(())
}

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<UserListItem>> {
    let items = sqlx::query_as::<_, UserListItem>(
        r#"
        SELECT
            u.id,
            u.username,
            u.email,
            u.role,
            u.is_active,
            u.permissions_json,
            u.parent_user_id,
            parent.username AS parent_username,
            u.created_by_user_id,
            creator.username AS creator_username,
            u.created_at,
            u.updated_at,
            COALESCE((
                SELECT COUNT(*)
                FROM users child
                WHERE child.parent_user_id = u.id
            ), 0) AS child_count,
            COALESCE((
                SELECT COUNT(*)
                FROM sessions s
                WHERE s.user_id = u.id
                  AND datetime(s.expires_at) > datetime('now')
            ), 0) AS active_session_count,
            (
                SELECT MAX(s.last_seen_at)
                FROM sessions s
                WHERE s.user_id = u.id
                  AND datetime(s.expires_at) > datetime('now')
            ) AS last_seen_at,
            COALESCE((SELECT COUNT(*) FROM sources src WHERE src.user_id = u.id), 0) AS source_count,
            COALESCE((
                SELECT COUNT(*)
                FROM discord_destinations dest
                WHERE dest.user_id = u.id
            ), 0) AS destination_count,
            COALESCE((
                SELECT COUNT(*)
                FROM message_templates tpl
                WHERE tpl.user_id = u.id
            ), 0) AS template_count,
            COALESCE((
                SELECT COUNT(*)
                FROM routing_rules rules
                WHERE rules.user_id = u.id
            ), 0) AS rule_count
        FROM users u
        LEFT JOIN users parent ON parent.id = u.parent_user_id
        LEFT JOIN users creator ON creator.id = u.created_by_user_id
        ORDER BY
            CASE u.role
                WHEN 'superadmin' THEN 0
                WHEN 'admin' THEN 1
                WHEN 'editor' THEN 2
                WHEN 'viewer' THEN 3
                ELSE 4
            END,
            lower(u.username) ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to list users")?;

    Ok(items)
}

pub async fn list_active_sessions(
    pool: &SqlitePool,
    limit: usize,
) -> Result<Vec<ActiveSessionListItem>> {
    let items = sqlx::query_as::<_, ActiveSessionListItem>(
        r#"
        SELECT
            s.id,
            u.id AS user_id,
            u.username,
            u.email,
            u.role,
            u.permissions_json,
            u.created_by_user_id,
            s.created_at,
            s.last_seen_at,
            s.expires_at,
            s.ip_address,
            s.user_agent
        FROM sessions s
        INNER JOIN users u ON u.id = s.user_id
        WHERE datetime(s.expires_at) > datetime('now')
        ORDER BY datetime(s.last_seen_at) DESC, datetime(s.created_at) DESC
        LIMIT ?
        "#,
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("failed to list active sessions")?;

    Ok(items)
}
