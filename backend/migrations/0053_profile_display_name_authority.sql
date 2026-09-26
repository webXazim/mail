-- Canonical account display-name authority.
--
-- Older frontend builds stored the Account > Display name field inside each
-- mailbox_settings JSON payload even though the signed-in account name lives
-- on users.display_name. That created two competing sources of truth: settings
-- could show one value while the sidebar/account menu rendered another.
--
-- Promote the most recently edited legacy mailbox preference only when it is
-- at least as new as the user row, then remove the legacy key from all mailbox
-- settings. Future clients update users.display_name via /api/profile and the
-- settings API strips displayName defensively.

WITH latest_legacy_name AS (
  SELECT DISTINCT ON (ms.user_id)
         ms.user_id,
         btrim(ms.payload ->> 'displayName') AS display_name,
         ms.updated_at
  FROM mailbox_settings ms
  WHERE jsonb_typeof(ms.payload) = 'object'
    AND ms.payload ? 'displayName'
    AND btrim(COALESCE(ms.payload ->> 'displayName', '')) <> ''
  ORDER BY ms.user_id, ms.updated_at DESC, ms.mailbox_id
)
UPDATE users u
SET display_name = legacy.display_name,
    updated_at = now()
FROM latest_legacy_name legacy
WHERE u.id = legacy.user_id
  AND legacy.updated_at >= u.updated_at
  AND u.display_name IS DISTINCT FROM legacy.display_name;

UPDATE mailbox_settings
SET payload = payload - 'displayName',
    updated_at = now()
WHERE jsonb_typeof(payload) = 'object'
  AND payload ? 'displayName';
