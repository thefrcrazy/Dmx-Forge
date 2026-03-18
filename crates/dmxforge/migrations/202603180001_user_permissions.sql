ALTER TABLE users ADD COLUMN permissions_json TEXT;

UPDATE sources
SET user_id = (
    SELECT id
    FROM users
    WHERE role IN ('superadmin', 'admin')
    ORDER BY created_at ASC
    LIMIT 1
)
WHERE user_id IS NULL;

UPDATE discord_destinations
SET user_id = (
    SELECT id
    FROM users
    WHERE role IN ('superadmin', 'admin')
    ORDER BY created_at ASC
    LIMIT 1
)
WHERE user_id IS NULL;

UPDATE message_templates
SET user_id = (
    SELECT id
    FROM users
    WHERE role IN ('superadmin', 'admin')
    ORDER BY created_at ASC
    LIMIT 1
)
WHERE user_id IS NULL;

UPDATE routing_rules
SET user_id = (
    SELECT id
    FROM users
    WHERE role IN ('superadmin', 'admin')
    ORDER BY created_at ASC
    LIMIT 1
)
WHERE user_id IS NULL;
