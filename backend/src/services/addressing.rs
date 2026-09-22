//! Upgrade 13 address authority.
//!
//! PostgreSQL stores the desired alias graph. Internal aliases are reconciled
//! into the destination mailbox Account aliases; external aliases are durable
//! one-recipient provider MailingList objects. Provider IO is retried from a
//! background worker so an upstream outage never loses an administrator's
//! requested address state.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::services::stalwart::{ManagedAccountAlias, ManagedMailingList, StalwartError};
use crate::state::AppState;

const MANAGED_PREFIX: &str = "CS Mail managed alias:";

#[derive(Debug, Clone, sqlx::FromRow)]
struct MailboxAliasRow {
    id: Uuid,
    domain: String,
    source: String,
    enabled: bool,
    deleted_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExternalAliasRow {
    id: Uuid,
    domain: String,
    source: String,
    dest_external: String,
    enabled: bool,
    provider_object_id: Option<String>,
    deleted_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

fn provider_error(error: StalwartError) -> String {
    match error {
        StalwartError::Rejected {
            operation,
            kind,
            description,
        } => format!("{operation}: {kind}: {description}"),
        other => other.to_string(),
    }
}

fn safe_error(error: &str) -> String {
    error.chars().take(1000).collect()
}

pub fn marker(id: Uuid) -> String {
    format!("{MANAGED_PREFIX}{id}")
}

pub async fn mark_mailbox_dirty(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<i64, String> {
    sqlx::query_scalar(
        "INSERT INTO address_sync_state(user_id, mailbox_id, desired_revision, status, next_attempt_at, updated_at)
         VALUES($1, $2, 1, 'pending', now(), now())
         ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET
           user_id=EXCLUDED.user_id,
           desired_revision = address_sync_state.desired_revision + 1,
           status='pending', last_error='', next_attempt_at=now(), updated_at=now()
         RETURNING desired_revision",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())
}

async fn account_for(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<String, String> {
    let row: Option<(String, Option<String>, String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT m.address::text,m.provider_account_id,m.local_part,m.provider_marker,d.provider_domain_id,d.is_system
         FROM mailboxes m
         JOIN organization_domains d ON d.id=m.domain_id
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
         WHERE m.id=$2 AND m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let (email,cached,local,marker,provider_domain_id,is_system) = row.ok_or_else(|| "destination business mailbox is not active".to_string())?;
    if !state.stalwart.enabled() { return Err("mail service is not configured".into()); }
    let account = if is_system {
        state.stalwart.find_account_by_email(&email).await.map_err(provider_error)?
    } else {
        let domain_id=provider_domain_id.as_deref().filter(|v|!v.trim().is_empty()).ok_or_else(||"destination mailbox provider domain binding is missing".to_string())?;
        state.stalwart.find_customer_account(domain_id,&local,&marker).await.map_err(provider_error)?
    }.ok_or_else(|| "destination mailbox is not provisioned".to_string())?;
    if let Some(cached)=cached.filter(|value| !value.trim().is_empty()) {
        if cached != account { return Err("stored provider account id does not match mailbox ownership".into()); }
    } else {
        let _ = sqlx::query("UPDATE mailboxes SET provider_account_id=$1,sync_status='ready',sync_error='',updated_at=now() WHERE id=$2")
            .bind(&account).bind(mailbox_id).execute(&state.db).await;
    }
    Ok(account)
}

async fn record_mailbox_error(state: &AppState, mailbox_id: Uuid, error: &str) {
    let safe = safe_error(error);
    let _ = sqlx::query(
        "UPDATE address_sync_state SET status='error',last_error=$2,attempts=attempts+1,
           next_attempt_at=now() + (LEAST(3600, 5 * power(2, LEAST(attempts, 9))::int) * interval '1 second'),
           updated_at=now() WHERE mailbox_id=$1",
    )
    .bind(mailbox_id)
    .bind(&safe)
    .execute(&state.db)
    .await;
    let _ = sqlx::query(
        "UPDATE aliases SET sync_status='error',sync_error=$2,sync_attempts=sync_attempts+1,
           next_attempt_at=now() + (LEAST(3600, 5 * power(2, LEAST(sync_attempts, 9))::int) * interval '1 second')
         WHERE dest_mailbox_id=$1 AND sync_status<>'deleted'",
    )
    .bind(mailbox_id)
    .bind(&safe)
    .execute(&state.db)
    .await;
}

pub async fn sync_mailbox_aliases(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<Value, String> {
    let mut lock = state.db.acquire().await.map_err(|e| e.to_string())?;
    let lock_key = format!("address:{mailbox_id}");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *lock)
        .await
        .map_err(|e| e.to_string())?;

    let result = sync_mailbox_aliases_locked(state, user_id, mailbox_id).await;
    if let Err(error) = result.as_ref() {
        record_mailbox_error(state, mailbox_id, error).await;
    }
    let _ = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *lock)
        .await;
    result
}

async fn sync_mailbox_aliases_locked(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<Value, String> {
    let revision: i64 = sqlx::query_scalar(
        "INSERT INTO address_sync_state(user_id,mailbox_id,status) VALUES($1,$2,'syncing')
         ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET user_id=EXCLUDED.user_id,status='syncing', updated_at=now()
         RETURNING desired_revision",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let account = account_for(state, user_id, mailbox_id).await?;
    let rows: Vec<MailboxAliasRow> = sqlx::query_as(
        "SELECT id,domain,source,enabled,deleted_at,updated_at
         FROM aliases WHERE dest_mailbox_id=$1 ORDER BY created_at,id",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let desired = rows
        .iter()
        .filter(|row| row.deleted_at.is_none())
        .map(|row| ManagedAccountAlias {
            marker: marker(row.id),
            local: row.source.clone(),
            domain: row.domain.clone(),
            enabled: row.enabled,
        })
        .collect::<Vec<_>>();

    state
        .stalwart
        .sync_account_aliases(&account, &desired)
        .await
        .map_err(provider_error)?;

    sqlx::query(
        "UPDATE address_sync_state SET applied_revision=$2,
           status=CASE WHEN desired_revision=$2 THEN 'ready' ELSE 'pending' END,
           last_error='', attempts=0, next_attempt_at=now(), synced_at=now(), updated_at=now()
         WHERE mailbox_id=$1",
    )
    .bind(mailbox_id)
    .bind(revision)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    // Acknowledge only the exact alias rows included in this reconciliation.
    // If an administrator changed a row while provider IO was in flight, its
    // updated_at no longer matches and it remains pending for the next pass.
    for row in &rows {
        let status = if row.deleted_at.is_some() {
            "deleted"
        } else {
            "ready"
        };
        sqlx::query(
            "UPDATE aliases SET sync_status=$3,sync_error='',sync_attempts=0,next_attempt_at=now(),synced_at=now()
             WHERE id=$1 AND updated_at=$2",
        )
        .bind(row.id)
        .bind(row.updated_at)
        .bind(status)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(json!({"status":"ready","revision":revision}))
}

pub async fn sync_external_alias(state: &AppState, alias_id: Uuid) -> Result<Value, String> {
    let mut lock = state.db.acquire().await.map_err(|e| e.to_string())?;
    let lock_key = format!("external-alias:{alias_id}");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *lock)
        .await
        .map_err(|e| e.to_string())?;
    let result = sync_external_alias_locked(state, alias_id).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *lock)
        .await;
    result
}

async fn sync_external_alias_locked(state: &AppState, alias_id: Uuid) -> Result<Value, String> {
    let row: Option<ExternalAliasRow> = sqlx::query_as(
        "UPDATE aliases SET sync_status='syncing'
         WHERE id=$1 AND dest_mailbox_id IS NULL AND sync_status IN ('pending','error')
         RETURNING id,domain,source,dest_external,enabled,provider_object_id,deleted_at,updated_at",
    )
    .bind(alias_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(json!({"status":"ready"}));
    };
    let marker = marker(row.id);
    let operation = if row.deleted_at.is_some() {
        state
            .stalwart
            .delete_managed_mailing_list(row.provider_object_id.as_deref(), &marker, &row.source)
            .await
            .map(|_| None)
    } else {
        state
            .stalwart
            .ensure_managed_mailing_list(
                row.provider_object_id.as_deref(),
                &ManagedMailingList {
                    marker,
                    local: row.source.clone(),
                    domain: row.domain.clone(),
                    recipient: row.dest_external.clone(),
                    enabled: row.enabled,
                },
            )
            .await
            .map(Some)
    };

    match operation {
        Ok(provider_id) => {
            let status = if row.deleted_at.is_some() {
                "deleted"
            } else {
                "ready"
            };
            sqlx::query(
                "UPDATE aliases SET provider_object_id=$3,sync_status=$4,sync_error='',sync_attempts=0,
                   next_attempt_at=now(),synced_at=now()
                 WHERE id=$1 AND updated_at=$2 AND sync_status='syncing'",
            )
            .bind(row.id)
            .bind(row.updated_at)
            .bind(provider_id)
            .bind(status)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            Ok(json!({"status":status}))
        }
        Err(error) => {
            let error = provider_error(error);
            let safe = safe_error(&error);
            let _ = sqlx::query(
                "UPDATE aliases SET sync_status='error',sync_error=$3,sync_attempts=sync_attempts+1,
                   next_attempt_at=now() + (LEAST(3600, 5 * power(2, LEAST(sync_attempts, 9))::int) * interval '1 second')
                 WHERE id=$1 AND updated_at=$2 AND sync_status='syncing'",
            )
            .bind(row.id)
            .bind(row.updated_at)
            .bind(&safe)
            .execute(&state.db)
            .await;
            Err(error)
        }
    }
}

pub async fn sync_after_alias_change(
    state: &AppState,
    alias_id: Uuid,
    destination: Option<(Uuid, Uuid)>,
) -> Value {
    let result = match destination {
        Some((user_id, mailbox_id)) => sync_mailbox_aliases(state, user_id, mailbox_id).await,
        None => sync_external_alias(state, alias_id).await,
    };
    match result {
        Ok(value) => value,
        Err(error) => json!({"status":"error","lastError":error}),
    }
}

pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let mailboxes: Vec<(Uuid, Uuid)> = match sqlx::query_as(
                "SELECT user_id, mailbox_id FROM address_sync_state
                 WHERE mailbox_id IS NOT NULL AND status IN ('pending','error') AND next_attempt_at<=now()
                 ORDER BY next_attempt_at,updated_at LIMIT 25",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, "address reconciliation user scan failed");
                    continue;
                }
            };
            for (user_id, mailbox_id) in mailboxes {
                if let Err(error) = sync_mailbox_aliases(&state, user_id, mailbox_id).await {
                    tracing::warn!(%user_id, %mailbox_id, %error, "mailbox alias reconciliation failed");
                }
            }
            let aliases: Vec<Uuid> = match sqlx::query_scalar(
                "SELECT id FROM aliases WHERE dest_mailbox_id IS NULL AND sync_status IN ('pending','error')
                 AND next_attempt_at<=now() ORDER BY next_attempt_at,created_at LIMIT 25",
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, "external alias reconciliation scan failed");
                    continue;
                }
            };
            for alias_id in aliases {
                if let Err(error) = sync_external_alias(&state, alias_id).await {
                    tracing::warn!(%alias_id, %error, "external alias reconciliation failed");
                }
            }
        }
    });
}
