CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    parent_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_by_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_users_parent_user_id ON users(parent_user_id);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ip_address TEXT,
    user_agent TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    webhook_secret TEXT,
    repository_filter TEXT,
    allowed_branches TEXT,
    allowed_events TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sources_provider_token ON sources(provider, token);
CREATE INDEX IF NOT EXISTS idx_sources_user_id ON sources(user_id);

CREATE TABLE IF NOT EXISTS discord_destinations (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    webhook_url TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_discord_destinations_user_id ON discord_destinations(user_id);

CREATE TABLE IF NOT EXISTS message_templates (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    format_style TEXT NOT NULL DEFAULT 'custom',
    body_template TEXT NOT NULL,
    embed_color TEXT,
    username_override TEXT,
    avatar_url_override TEXT,
    footer_text TEXT,
    show_avatar INTEGER NOT NULL DEFAULT 1,
    show_repo_link INTEGER NOT NULL DEFAULT 1,
    show_branch INTEGER NOT NULL DEFAULT 1,
    show_commits INTEGER NOT NULL DEFAULT 1,
    show_status_badge INTEGER NOT NULL DEFAULT 1,
    show_timestamp INTEGER NOT NULL DEFAULT 1,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_message_templates_user_id ON message_templates(user_id);

CREATE TABLE IF NOT EXISTS routing_rules (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    source_id TEXT REFERENCES sources(id) ON DELETE SET NULL,
    destination_id TEXT NOT NULL REFERENCES discord_destinations(id) ON DELETE CASCADE,
    template_id TEXT NOT NULL REFERENCES message_templates(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    provider_filter TEXT,
    event_type_filter TEXT,
    branch_prefix_filter TEXT,
    repository_filter TEXT,
    skip_keyword TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_routing_rules_user_id ON routing_rules(user_id);
CREATE INDEX IF NOT EXISTS idx_routing_rules_sort_order ON routing_rules(sort_order);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id TEXT PRIMARY KEY,
    source_id TEXT REFERENCES sources(id) ON DELETE SET NULL,
    provider TEXT NOT NULL,
    event_type TEXT,
    repository TEXT,
    branch TEXT,
    status TEXT NOT NULL,
    failure_reason TEXT,
    raw_headers TEXT NOT NULL,
    raw_payload TEXT NOT NULL,
    normalized_event TEXT,
    received_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_status ON webhook_deliveries(status);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_provider ON webhook_deliveries(provider);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_repository ON webhook_deliveries(repository);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_received_at ON webhook_deliveries(received_at);

CREATE TABLE IF NOT EXISTS discord_messages (
    id TEXT PRIMARY KEY,
    delivery_id TEXT NOT NULL REFERENCES webhook_deliveries(id) ON DELETE CASCADE,
    destination_id TEXT REFERENCES discord_destinations(id) ON DELETE SET NULL,
    request_payload TEXT NOT NULL,
    response_status INTEGER,
    response_body TEXT,
    status TEXT NOT NULL,
    attempted_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_discord_messages_delivery_id ON discord_messages(delivery_id);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_user_id ON audit_logs(actor_user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON audit_logs(action);

CREATE TABLE IF NOT EXISTS user_shared_configs (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    share_token TEXT NOT NULL UNIQUE,
    payload_json TEXT NOT NULL,
    expires_at TEXT,
    is_single_use INTEGER NOT NULL DEFAULT 0,
    redeemed_at TEXT,
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_user_shared_configs_owner_user_id ON user_shared_configs(owner_user_id);
