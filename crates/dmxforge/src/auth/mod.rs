use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{
        HeaderMap, StatusCode,
        header::{COOKIE, HeaderValue, LOCATION, SET_COOKIE},
        request::Parts,
    },
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, db};

const GUEST_CSRF_COOKIE_SUFFIX: &str = "__guest_csrf";
const SESSION_TOUCH_INTERVAL_MINUTES: i64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionSet {
    pub sources_read: bool,
    pub sources_write: bool,
    pub destinations_read: bool,
    pub destinations_write: bool,
    pub templates_read: bool,
    pub templates_write: bool,
    pub rules_read: bool,
    pub rules_write: bool,
    pub deliveries_read: bool,
    pub deliveries_replay: bool,
    pub users_read: bool,
    pub users_write: bool,
    pub subusers_create: bool,
}

impl PermissionSet {
    pub fn all() -> Self {
        Self {
            sources_read: true,
            sources_write: true,
            destinations_read: true,
            destinations_write: true,
            templates_read: true,
            templates_write: true,
            rules_read: true,
            rules_write: true,
            deliveries_read: true,
            deliveries_replay: true,
            users_read: true,
            users_write: true,
            subusers_create: true,
        }
    }

    pub fn for_role(role: &str) -> Self {
        match role {
            "superadmin" => Self::all(),
            "admin" => Self {
                deliveries_replay: true,
                users_write: true,
                subusers_create: true,
                ..Self::editor_defaults()
            },
            "editor" => Self::editor_defaults(),
            "viewer" => Self::viewer_defaults(),
            _ => Self::default(),
        }
    }

    pub fn from_json_or_role(role: &str, raw: Option<&str>) -> Self {
        let baseline = Self::for_role(role);
        let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
            return baseline;
        };

        match serde_json::from_str::<Self>(raw) {
            Ok(parsed) => parsed.intersect(&baseline),
            Err(_) => baseline,
        }
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        serde_json::to_string(self)
            .map_err(|error| anyhow::anyhow!("failed to serialize permission set: {error}"))
    }

    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            sources_read: self.sources_read && other.sources_read,
            sources_write: self.sources_write && other.sources_write,
            destinations_read: self.destinations_read && other.destinations_read,
            destinations_write: self.destinations_write && other.destinations_write,
            templates_read: self.templates_read && other.templates_read,
            templates_write: self.templates_write && other.templates_write,
            rules_read: self.rules_read && other.rules_read,
            rules_write: self.rules_write && other.rules_write,
            deliveries_read: self.deliveries_read && other.deliveries_read,
            deliveries_replay: self.deliveries_replay && other.deliveries_replay,
            users_read: self.users_read && other.users_read,
            users_write: self.users_write && other.users_write,
            subusers_create: self.subusers_create && other.subusers_create,
        }
    }

    fn editor_defaults() -> Self {
        Self {
            sources_read: true,
            sources_write: true,
            destinations_read: true,
            destinations_write: true,
            templates_read: true,
            templates_write: true,
            rules_read: true,
            rules_write: true,
            deliveries_read: true,
            deliveries_replay: false,
            users_read: true,
            users_write: false,
            subusers_create: false,
        }
    }

    fn viewer_defaults() -> Self {
        Self {
            sources_read: true,
            sources_write: false,
            destinations_read: true,
            destinations_write: false,
            templates_read: true,
            templates_write: false,
            rules_read: true,
            rules_write: false,
            deliveries_read: true,
            deliveries_replay: false,
            users_read: true,
            users_write: false,
            subusers_create: false,
        }
    }
}

impl Default for PermissionSet {
    fn default() -> Self {
        Self::viewer_defaults()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role_label: String,
    pub csrf_token: String,
    pub is_authenticated: bool,
    pub permissions: PermissionSet,
}

impl CurrentUser {
    pub fn guest() -> Self {
        Self {
            id: String::new(),
            username: "Guest".to_string(),
            email: String::new(),
            role_label: "visitor".to_string(),
            csrf_token: String::new(),
            is_authenticated: false,
            permissions: PermissionSet::default(),
        }
    }

    pub fn is_admin(&self) -> bool {
        is_admin_role(&self.role_label)
    }

    pub fn is_superadmin(&self) -> bool {
        self.role_label == "superadmin"
    }

    pub fn role_rank(&self) -> u8 {
        role_rank(&self.role_label)
    }

    pub fn can_read_sources(&self) -> bool {
        self.permissions.sources_read
    }

    pub fn can_write_sources(&self) -> bool {
        self.permissions.sources_write
    }

    pub fn can_read_destinations(&self) -> bool {
        self.permissions.destinations_read
    }

    pub fn can_write_destinations(&self) -> bool {
        self.permissions.destinations_write
    }

    pub fn can_read_templates(&self) -> bool {
        self.permissions.templates_read
    }

    pub fn can_write_templates(&self) -> bool {
        self.permissions.templates_write
    }

    pub fn can_read_rules(&self) -> bool {
        self.permissions.rules_read
    }

    pub fn can_write_rules(&self) -> bool {
        self.permissions.rules_write
    }

    pub fn can_read_deliveries(&self) -> bool {
        self.permissions.deliveries_read
    }

    pub fn can_replay_deliveries(&self) -> bool {
        self.permissions.deliveries_replay
    }

    pub fn can_read_users(&self) -> bool {
        self.permissions.users_read
    }

    pub fn can_write_users(&self) -> bool {
        self.permissions.users_write
    }

    pub fn can_create_subusers(&self) -> bool {
        self.permissions.subusers_create
    }
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub current_user: CurrentUser,
    pub session_id: String,
    pub csrf_token: String,
}

impl<S> FromRequestParts<S> for AuthSession
where
    std::sync::Arc<AppState>: axum::extract::FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = std::sync::Arc::<AppState>::from_ref(state);

        match load_auth_session_from_headers(&state, &parts.headers).await {
            Ok(Some(session)) => Ok(session),
            Ok(None) => {
                if db::admin_user_exists(&state.db).await.unwrap_or(false) {
                    Err(redirect_to_login(None))
                } else {
                    Err(Redirect::to("/setup").into_response())
                }
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to resolve auth session");
                Err((StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response())
            }
        }
    }
}

pub async fn load_auth_session_from_headers(
    state: &std::sync::Arc<AppState>,
    headers: &HeaderMap,
) -> anyhow::Result<Option<AuthSession>> {
    let Some(session_id) = session_cookie_value(headers, &state.config.session_cookie_name) else {
        return Ok(None);
    };

    let Some(session) = db::find_session_user(&state.db, &session_id).await? else {
        return Ok(None);
    };

    if session.is_active == 0 {
        db::delete_session(&state.db, &session_id).await?;
        return Ok(None);
    }

    if should_touch_session(&session.last_seen_at) {
        db::touch_session(&state.db, &session_id).await?;
    }

    let permissions =
        PermissionSet::from_json_or_role(&session.role, session.permissions_json.as_deref());

    Ok(Some(AuthSession {
        current_user: CurrentUser {
            id: session.user_id.clone(),
            username: session.username,
            email: session.email,
            role_label: session.role,
            csrf_token: session.csrf_token.clone(),
            is_authenticated: true,
            permissions,
        },
        session_id,
        csrf_token: session.csrf_token,
    }))
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
        .map_err(|error| anyhow::anyhow!("failed to generate password salt: {error}"))?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, password_hash: &str) -> anyhow::Result<bool> {
    let parsed = PasswordHash::new(password_hash)
        .map_err(|error| anyhow::anyhow!("failed to parse password hash: {error}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub fn is_admin_role(role: &str) -> bool {
    matches!(role, "superadmin" | "admin")
}

pub fn role_rank(role: &str) -> u8 {
    match role {
        "superadmin" => 3,
        "admin" => 2,
        "editor" => 1,
        "viewer" => 0,
        _ => 0,
    }
}

pub fn new_guest_csrf_token() -> String {
    Uuid::new_v4().simple().to_string()
}

pub fn guest_csrf_cookie(state: &AppState, token: &str) -> HeaderValue {
    HeaderValue::from_str(&guest_csrf_cookie_value_string(state, token, Some(15 * 60)))
        .expect("valid guest csrf cookie header")
}

pub fn clearing_guest_csrf_cookie(state: &AppState) -> HeaderValue {
    HeaderValue::from_str(&guest_csrf_cookie_value_string(state, "", Some(0)))
        .expect("valid clearing guest csrf cookie header")
}

pub fn guest_csrf_matches(state: &AppState, headers: &HeaderMap, submitted_token: &str) -> bool {
    let cookie_name = guest_csrf_cookie_name(state);
    !submitted_token.trim().is_empty()
        && session_cookie_value(headers, &cookie_name).as_deref() == Some(submitted_token)
}

pub fn new_session_cookie(state: &AppState, session_id: &str) -> HeaderValue {
    HeaderValue::from_str(&session_cookie_value_string(
        state,
        session_id,
        Some((state.config.session_ttl_hours * 3600) as i64),
    ))
    .expect("valid session cookie header")
}

pub fn clearing_session_cookie(state: &AppState) -> HeaderValue {
    HeaderValue::from_str(&session_cookie_value_string(state, "", Some(0)))
        .expect("valid clearing session cookie header")
}

pub fn redirect_with_cookie(location: &str, cookie: HeaderValue) -> Response {
    let mut response = Redirect::to(location).into_response();
    response.headers_mut().insert(SET_COOKIE, cookie);
    response
}

pub fn redirect_to_login(cookie: Option<HeaderValue>) -> Response {
    let mut response = Redirect::to("/login").into_response();
    if let Some(cookie) = cookie {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static("/login"));
    response
}

pub fn validate_password_rules(password: &str, confirmation: &str) -> Result<(), String> {
    if password.len() < 12 {
        return Err("Password must contain at least 12 characters.".to_string());
    }

    if password != confirmation {
        return Err("Password confirmation does not match.".to_string());
    }

    Ok(())
}

pub fn session_expires_at(hours: u64) -> String {
    (Utc::now() + Duration::hours(hours as i64)).to_rfc3339()
}

fn should_touch_session(last_seen_at: &str) -> bool {
    let cutoff = Utc::now().naive_utc() - Duration::minutes(SESSION_TOUCH_INTERVAL_MINUTES);

    match NaiveDateTime::parse_from_str(last_seen_at, "%Y-%m-%d %H:%M:%S") {
        Ok(last_seen_at) => last_seen_at <= cutoff,
        Err(_) => true,
    }
}

pub fn session_cookie_value(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;

    raw.split(';').find_map(|segment| {
        let mut parts = segment.trim().splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        (key == cookie_name).then(|| value.to_string())
    })
}

pub fn client_ip_from_headers(headers: &HeaderMap) -> Option<String> {
    for name in ["x-forwarded-for", "x-real-ip"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let ip = value.split(',').next().unwrap_or(value).trim();
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }

    None
}

pub fn user_agent_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn session_cookie_value_string(
    state: &AppState,
    session_id: &str,
    max_age_seconds: Option<i64>,
) -> String {
    cookie_value_string(
        &state.config.session_cookie_name,
        session_id,
        max_age_seconds,
        state.config.secure_cookies,
    )
}

fn guest_csrf_cookie_name(state: &AppState) -> String {
    format!(
        "{}{}",
        state.config.session_cookie_name, GUEST_CSRF_COOKIE_SUFFIX
    )
}

fn guest_csrf_cookie_value_string(
    state: &AppState,
    token: &str,
    max_age_seconds: Option<i64>,
) -> String {
    cookie_value_string(
        &guest_csrf_cookie_name(state),
        token,
        max_age_seconds,
        state.config.secure_cookies,
    )
}

fn cookie_value_string(
    cookie_name: &str,
    value: &str,
    max_age_seconds: Option<i64>,
    secure: bool,
) -> String {
    let mut parts = vec![
        format!("{cookie_name}={value}"),
        "Path=/".to_string(),
        "HttpOnly".to_string(),
        "SameSite=Lax".to_string(),
    ];

    if let Some(max_age) = max_age_seconds {
        parts.push(format!("Max-Age={max_age}"));
    }

    if secure {
        parts.push("Secure".to_string());
    }

    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn hashes_and_verifies_passwords() {
        let password = "a-very-long-password";
        let hash = hash_password(password).expect("hash must succeed");

        assert_ne!(hash, password);
        assert!(
            verify_password(password, &hash).expect("verification must succeed"),
            "password should match its hash"
        );
        assert!(
            !verify_password("wrong-password", &hash).expect("verification must succeed"),
            "different passwords should not verify"
        );
    }

    #[test]
    fn validates_password_policy() {
        assert!(validate_password_rules("0123456789ab", "0123456789ab").is_ok());
        assert!(validate_password_rules("short", "short").is_err());
        assert!(validate_password_rules("0123456789ab", "different-value").is_err());
    }

    #[test]
    fn extracts_session_cookie_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("foo=bar; dmxforge_session=session-123; another=value"),
        );

        assert_eq!(
            session_cookie_value(&headers, "dmxforge_session").as_deref(),
            Some("session-123")
        );
        assert!(session_cookie_value(&headers, "missing").is_none());
    }

    #[test]
    fn extracts_forwarded_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.10, 10.0.0.1"),
        );

        assert_eq!(
            client_ip_from_headers(&headers).as_deref(),
            Some("203.0.113.10")
        );
    }

    #[test]
    fn recognizes_admin_roles() {
        assert!(is_admin_role("superadmin"));
        assert!(is_admin_role("admin"));
        assert!(!is_admin_role("editor"));
        assert!(!is_admin_role("viewer"));
    }

    #[test]
    fn applies_role_defaults_to_permissions() {
        let admin = PermissionSet::for_role("admin");
        let viewer = PermissionSet::for_role("viewer");

        assert!(admin.users_write);
        assert!(admin.subusers_create);
        assert!(!viewer.sources_write);
        assert!(viewer.sources_read);
    }

    #[test]
    fn delegated_permissions_cannot_exceed_role_defaults() {
        let delegated = PermissionSet {
            deliveries_replay: true,
            users_write: true,
            ..PermissionSet::all()
        };
        let editor = delegated.intersect(&PermissionSet::for_role("editor"));

        assert!(!editor.deliveries_replay);
        assert!(!editor.users_write);
        assert!(editor.sources_write);
    }

    #[test]
    fn touches_stale_sessions_only() {
        let fresh = Utc::now()
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let stale = (Utc::now() - Duration::minutes(SESSION_TOUCH_INTERVAL_MINUTES + 1))
            .naive_utc()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        assert!(!should_touch_session(&fresh));
        assert!(should_touch_session(&stale));
        assert!(should_touch_session("invalid-timestamp"));
    }

    #[tokio::test]
    async fn validates_guest_csrf_cookie_against_form_token() {
        let state = AppState {
            config: crate::config::AppConfig {
                app_name: "DmxForge".to_string(),
                bind_address: "0.0.0.0".to_string(),
                port: 3000,
                database_url: "sqlite::memory:".to_string(),
                database_max_connections: 1,
                session_cookie_name: "dmxforge_session".to_string(),
                session_ttl_hours: 24,
                secure_cookies: false,
                payload_limit_kb: 512,
                secret_key: "0123456789abcdef0123456789abcdef".to_string(),
            },
            db: sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
            discord: crate::discord::DiscordTemplateEngine::new(),
            http_client: reqwest::Client::new(),
            login_rate_limit: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };
        let token = "guest-token-123";
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("dmxforge_session__guest_csrf=guest-token-123"),
        );

        assert!(guest_csrf_matches(&state, &headers, token));
        assert!(!guest_csrf_matches(&state, &headers, "other-token"));
    }
}
