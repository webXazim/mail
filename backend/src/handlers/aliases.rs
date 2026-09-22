use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AdminUser;
use crate::services::{addressing, automation};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AliasIn {
    domain: String,
    source: String,
    #[serde(rename = "forwardTo")]
    forward_to: String,
}

#[derive(sqlx::FromRow)]
struct AliasRow {
    id: Uuid,
    domain: String,
    source: String,
    dest_user: Option<Uuid>,
    dest_mailbox_id: Option<Uuid>,
    #[sqlx(rename = "forward_email")]
    forward_email: Option<String>,
    dest_external: String,
    enabled: bool,
    sync_status: String,
    sync_error: String,
    synced_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn row_to_json(row: &AliasRow) -> Value {
    let address = format!("{}@{}", row.source, row.domain);
    let forward = row
        .forward_email
        .clone()
        .unwrap_or_else(|| row.dest_external.clone());
    json!({
        "id": row.id,
        "address": address,
        "domain": row.domain,
        "source": row.source,
        "forwardTo": forward,
        "destinationType": if row.dest_mailbox_id.is_some() { "mailbox" } else { "external" },
        "enabled": row.enabled,
        "syncStatus": row.sync_status,
        "syncError": row.sync_error,
        "syncedAt": row.synced_at,
    })
}

/// Active aliases with their resolved destination. Deleted aliases remain as
/// provider-reconciliation tombstones and are intentionally hidden here.
pub async fn all(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<AliasRow> = sqlx::query_as(
        "SELECT a.id,a.domain,a.source,a.dest_user,a.dest_mailbox_id,m.address::text AS forward_email,a.dest_external,
                a.enabled,a.sync_status,a.sync_error,a.synced_at
         FROM aliases a
         LEFT JOIN mailboxes m ON m.id=a.dest_mailbox_id
         WHERE a.deleted_at IS NULL
         ORDER BY a.domain,a.source",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows.iter().map(row_to_json).collect())
}

pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "aliases": all(&state).await? })))
}

fn valid_local(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
}

async fn would_create_alias_loop(
    state: &AppState,
    new_address: &str,
    forward_to: &str,
) -> Result<bool, ApiError> {
    if new_address.eq_ignore_ascii_case(forward_to) {
        return Ok(true);
    }

    // Follow only active external aliases. Internal mailbox destinations end
    // the route and therefore cannot form an alias-to-alias delivery cycle.
    // The path array makes the query safe even if legacy data already contains
    // a cycle, while the depth cap bounds work on damaged/imported datasets.
    sqlx::query_scalar(
        r#"WITH RECURSIVE chain(address, path, depth) AS (
             SELECT lower($1::text), ARRAY[lower($1::text)], 0
             UNION ALL
             SELECT lower(a.dest_external), c.path || lower(a.dest_external), c.depth + 1
             FROM chain c
             JOIN aliases a
               ON lower(a.source || '@' || a.domain) = c.address
              AND a.dest_mailbox_id IS NULL
              AND a.deleted_at IS NULL
              AND a.enabled = TRUE
             WHERE c.depth < 50
               AND NOT lower(a.dest_external) = ANY(c.path)
           )
           SELECT EXISTS(
             SELECT 1 FROM chain WHERE address = lower($2::text)
           )"#,
    )
    .bind(forward_to)
    .bind(new_address)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn create(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<AliasIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let domain = body.domain.trim().to_lowercase();
    let source = body.source.trim().to_lowercase();
    let forward = body.forward_to.trim().to_lowercase();

    if domain.is_empty() || source.is_empty() || forward.is_empty() {
        return Err(ApiError::bad_request("domain, source and forwardTo are required"));
    }
    if !domain.eq_ignore_ascii_case(state.stalwart.default_domain()) {
        return Err(ApiError::bad_request("Alias domain is not managed by CS Mail"));
    }
    if !valid_local(&source) {
        return Err(ApiError::bad_request(
            "source must be a valid email local part using letters, digits, '.', '-' or '_'",
        ));
    }
    if !automation::valid_email(&forward) {
        return Err(ApiError::bad_request("forwardTo must be a valid email address"));
    }
    let address = format!("{source}@{domain}");
    if would_create_alias_loop(&state, &address, &forward).await? {
        return Err(ApiError::bad_request(
            "That destination would create an alias forwarding loop",
        ));
    }
    let mailbox_conflict: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE lower(address::text)=lower($1) AND deleted_at IS NULL)",
    )
    .bind(&address)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if mailbox_conflict {
        return Err(ApiError::conflict("That address is already a mailbox"));
    }

    let destination: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT m.id,m.user_id,COALESCE(NULLIF(m.display_name,''),u.display_name)
         FROM mailboxes m
         JOIN users u ON u.id=m.user_id AND u.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=m.user_id AND om.status='active'
         WHERE lower(m.address::text)=lower($1) AND m.deleted_at IS NULL AND m.status='active'",
    )
    .bind(&forward)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: AliasRow = sqlx::query_as(
        "INSERT INTO aliases(domain,source,dest_user,dest_mailbox_id,dest_external,enabled,sync_status,next_attempt_at,updated_at)
         VALUES($1,$2,$3,$4,$5,TRUE,'pending',now(),now())
         RETURNING id,domain,source,dest_user,dest_mailbox_id,
           (SELECT m.address::text FROM mailboxes m WHERE m.id=aliases.dest_mailbox_id) AS forward_email,
           dest_external,enabled,sync_status,sync_error,synced_at",
    )
    .bind(&domain)
    .bind(&source)
    .bind(destination.as_ref().map(|(_, user_id, _)| *user_id))
    .bind(destination.as_ref().map(|(mailbox_id, _, _)| *mailbox_id))
    .bind(if destination.is_some() { "" } else { &forward })
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db) = &e {
            if db.code().as_deref() == Some("23505") {
                return ApiError::conflict(format!("{address} already exists"));
            }
        }
        ApiError::internal(e.to_string())
    })?;

    if let Some((mailbox_id, user_id, display_name)) = destination.as_ref() {
        sqlx::query(
            "INSERT INTO sender_identities
              (user_id,mailbox_id,email,display_name,is_primary,status,source,alias_id,verified_at)
             VALUES($1,$2,$3,$4,FALSE,'verified','alias',$5,now())
             ON CONFLICT(mailbox_id,email) DO UPDATE SET
               source='alias',alias_id=EXCLUDED.alias_id,
               status=CASE WHEN sender_identities.status='disabled' THEN 'disabled' ELSE 'verified' END,
               verified_at=COALESCE(sender_identities.verified_at,now()),updated_at=now()",
        )
        .bind(user_id)
        .bind(mailbox_id)
        .bind(&address)
        .bind(if display_name.trim().is_empty() { &source } else { display_name })
        .bind(row.id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO address_sync_state(user_id,mailbox_id,desired_revision,status,next_attempt_at,updated_at)
             VALUES($1,$2,1,'pending',now(),now())
             ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET user_id=EXCLUDED.user_id, desired_revision=address_sync_state.desired_revision+1,
               status='pending',last_error='',next_attempt_at=now(),updated_at=now()",
        )
        .bind(user_id)
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let sync = addressing::sync_after_alias_change(&state, row.id, row.dest_user.zip(row.dest_mailbox_id)).await;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "alias.create",
        json!({ "address": address, "forwardTo": forward, "destinationType": if row.dest_mailbox_id.is_some() {"mailbox"} else {"external"} }),
    )
    .await;

    let refreshed: AliasRow = sqlx::query_as(
        "SELECT a.id,a.domain,a.source,a.dest_user,a.dest_mailbox_id,m.address::text AS forward_email,a.dest_external,
                a.enabled,a.sync_status,a.sync_error,a.synced_at
         FROM aliases a LEFT JOIN mailboxes m ON m.id=a.dest_mailbox_id WHERE a.id=$1",
    )
    .bind(row.id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((axum::http::StatusCode::CREATED, Json(json!({"alias":row_to_json(&refreshed),"sync":sync}))))
}

pub async fn delete(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let current: Option<(Option<Uuid>, Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT dest_user,dest_mailbox_id,domain,source FROM aliases WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((dest_user, dest_mailbox_id, domain, source)) = current else {
        return Err(ApiError::not_found("Alias not found"));
    };

    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE aliases SET deleted_at=now(),enabled=FALSE,sync_status='pending',sync_error='',
          next_attempt_at=now(),updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if let (Some(user_id), Some(mailbox_id)) = (dest_user, dest_mailbox_id) {
        let was_default: Option<bool> = sqlx::query_scalar(
            "SELECT is_primary FROM sender_identities WHERE alias_id=$1 AND mailbox_id=$2",
        )
        .bind(id)
        .bind(mailbox_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query(
            "UPDATE sender_identities SET status='disabled',is_primary=FALSE,updated_at=now()
             WHERE alias_id=$1 AND mailbox_id=$2",
        )
        .bind(id)
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if was_default == Some(true) {
            sqlx::query(
                "UPDATE sender_identities SET is_primary=TRUE,updated_at=now()
                 WHERE id=(SELECT id FROM sender_identities
                           WHERE mailbox_id=$1 AND source='primary' AND status='verified'
                           ORDER BY created_at LIMIT 1)",
            )
            .bind(mailbox_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        }
        sqlx::query(
            "INSERT INTO address_sync_state(user_id,mailbox_id,desired_revision,status,next_attempt_at,updated_at)
             VALUES($1,$2,1,'pending',now(),now())
             ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET user_id=EXCLUDED.user_id, desired_revision=address_sync_state.desired_revision+1,
              status='pending',last_error='',next_attempt_at=now(),updated_at=now()",
        )
        .bind(user_id)
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let sync = addressing::sync_after_alias_change(&state, id, dest_user.zip(dest_mailbox_id)).await;
    audit::record(
        &state,
        Some(admin.0.user_id),
        "alias.delete",
        json!({ "id": id, "address": format!("{source}@{domain}") }),
    )
    .await;
    Ok(Json(json!({ "ok": true, "sync": sync })))
}
