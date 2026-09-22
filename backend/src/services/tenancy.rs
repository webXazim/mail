use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PrimaryMailbox {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub domain_id: Uuid,
    pub address: String,
    pub display_name: String,
    pub status: String,
    pub provider_account_id: Option<String>,
    pub sync_status: String,
    pub quota_bytes: i64,
}

pub type ActiveMailbox = PrimaryMailbox;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Membership {
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub status: String,
}

/// Resolve a concrete mailbox for a platform user. Header/request hints are
/// authoritative only after verifying active organization membership and that
/// the mailbox is assigned to this exact user. This is the central IDOR guard
/// for every mailbox-scoped feature introduced in Upgrade 23.
pub async fn active_mailbox(
    pool: &PgPool,
    user_id: Uuid,
    organization_hint: Option<Uuid>,
    mailbox_hint: Option<Uuid>,
) -> Result<Option<ActiveMailbox>, ApiError> {
    if let Some(mailbox_id) = mailbox_hint {
        let row = mailbox_by_id(pool, user_id, mailbox_id).await?;
        let Some(mailbox) = row else {
            return Err(ApiError::not_found("Mailbox not found"));
        };
        if let Some(organization_id) = organization_hint {
            if mailbox.organization_id != organization_id {
                return Err(ApiError::forbidden("Mailbox does not belong to the selected business"));
            }
        }
        if mailbox.status != "active" {
            return Err(ApiError::forbidden("The selected mailbox is not active"));
        }
        return Ok(Some(mailbox));
    }

    // Persisted active selection is the default for clients that do not yet
    // send explicit context headers. primary_mailbox_id remains a compatibility
    // fallback for older rows until all deployments have run migration 0030.
    let selected: (Option<Uuid>, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_organization_id, active_mailbox_id, primary_mailbox_id
         FROM users WHERE id=$1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let organization_id = organization_hint.or(selected.0);
    if let Some(mailbox_id) = selected.1.or(selected.2) {
        if let Some(mailbox) = mailbox_by_id(pool, user_id, mailbox_id).await? {
            if mailbox.status == "active"
                && organization_id.map(|id| id == mailbox.organization_id).unwrap_or(true)
            {
                return Ok(Some(mailbox));
            }
        }
    }

    // If the selected organization changed, choose one assigned mailbox from
    // that organization rather than accidentally retaining the old tenant.
    if let Some(org_id) = organization_id {
        let row = sqlx::query_as::<_, ActiveMailbox>(
            "SELECT m.id,m.organization_id,m.domain_id,m.address::text,m.display_name,
                    m.status,m.provider_account_id,m.sync_status,m.quota_bytes
             FROM mailboxes m
             JOIN organization_memberships om
               ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
             JOIN organizations o ON o.id=m.organization_id AND o.status='active'
             WHERE m.organization_id=$2 AND m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'
             ORDER BY m.is_primary_for_user DESC,
                      CASE m.status WHEN 'active' THEN 0 WHEN 'pending' THEN 1 ELSE 2 END,
                      m.created_at ASC
             LIMIT 1",
        )
        .bind(user_id)
        .bind(org_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok(row);
    }

    sqlx::query_as::<_, ActiveMailbox>(
        "SELECT m.id,m.organization_id,m.domain_id,m.address::text,m.display_name,
                m.status,m.provider_account_id,m.sync_status,m.quota_bytes
         FROM mailboxes m
         JOIN organization_memberships om
           ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         WHERE m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'
         ORDER BY m.is_primary_for_user DESC,
                  CASE m.status WHEN 'active' THEN 0 WHEN 'pending' THEN 1 ELSE 2 END,
                  m.created_at ASC
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

async fn mailbox_by_id(
    pool: &PgPool,
    user_id: Uuid,
    mailbox_id: Uuid,
) -> Result<Option<ActiveMailbox>, ApiError> {
    sqlx::query_as::<_, ActiveMailbox>(
        "SELECT m.id,m.organization_id,m.domain_id,m.address::text,m.display_name,
                m.status,m.provider_account_id,m.sync_status,m.quota_bytes
         FROM mailboxes m
         JOIN organization_memberships om
           ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         WHERE m.id=$2 AND m.user_id=$1 AND m.deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

/// Compatibility helper for background jobs that do not carry request headers.
/// New request handlers should call `active_mailbox` with AuthUser hints.
pub async fn primary_mailbox(pool: &PgPool, user_id: Uuid) -> Result<Option<PrimaryMailbox>, ApiError> {
    active_mailbox(pool, user_id, None, None).await
}

pub async fn set_active_mailbox(
    pool: &PgPool,
    user_id: Uuid,
    organization_id: Uuid,
    mailbox_id: Uuid,
) -> Result<ActiveMailbox, ApiError> {
    let mailbox = active_mailbox(pool, user_id, Some(organization_id), Some(mailbox_id))
        .await?
        .ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    if mailbox.status != "active" {
        return Err(ApiError::forbidden("Only an active mailbox can be selected"));
    }
    sqlx::query(
        "UPDATE users SET active_organization_id=$1,active_mailbox_id=$2,updated_at=now() WHERE id=$3",
    )
    .bind(organization_id)
    .bind(mailbox_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(mailbox)
}

pub async fn membership(
    pool: &PgPool,
    user_id: Uuid,
    organization_id: Uuid,
) -> Result<Option<Membership>, ApiError> {
    sqlx::query_as::<_, Membership>(
        "SELECT organization_id, user_id, role, status
         FROM organization_memberships
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn require_member(
    pool: &PgPool,
    user_id: Uuid,
    organization_id: Uuid,
) -> Result<Membership, ApiError> {
    let membership = membership(pool, user_id, organization_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Business not found"))?;
    if membership.status != "active" {
        return Err(ApiError::forbidden("Your business membership is not active"));
    }
    Ok(membership)
}

pub async fn require_admin(
    pool: &PgPool,
    user_id: Uuid,
    organization_id: Uuid,
) -> Result<Membership, ApiError> {
    let membership = require_member(pool, user_id, organization_id).await?;
    if !matches!(membership.role.as_str(), "owner" | "admin") {
        return Err(ApiError::forbidden("Business owner or administrator access required"));
    }
    Ok(membership)
}

pub async fn require_owner(
    pool: &PgPool,
    user_id: Uuid,
    organization_id: Uuid,
) -> Result<Membership, ApiError> {
    let membership = require_member(pool, user_id, organization_id).await?;
    if membership.role != "owner" {
        return Err(ApiError::forbidden("Business owner access required"));
    }
    Ok(membership)
}
