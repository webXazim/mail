use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::handlers::auth::valid_email;
use crate::middleware::auth::AuthUser;
use crate::services::{email, tenancy};
use crate::state::AppState;

const INVITE_TTL_DAYS: i64 = 7;

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn normalize_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Business name is required"));
    }
    if name.chars().count() > 120 {
        return Err(ApiError::bad_request("Business name is too long"));
    }
    Ok(name.to_string())
}

fn slug_from_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 56 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() < 3 {
        "business".to_string()
    } else {
        out
    }
}

async fn unique_slug(state: &AppState, name: &str) -> Result<String, ApiError> {
    let base = slug_from_name(name);
    for attempt in 0..16 {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            format!("{}-{}", base, &Uuid::new_v4().as_simple().to_string()[..6])
        };
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organizations WHERE lower(slug) = lower($1))",
        )
        .bind(&candidate)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict("Could not allocate a unique business slug"))
}

#[derive(Debug, sqlx::FromRow)]
struct OrganizationRow {
    id: Uuid,
    name: String,
    slug: String,
    status: String,
    is_system: bool,
    role: String,
    member_count: i64,
    domain_count: i64,
    active_domain_count: i64,
    mailbox_count: i64,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<OrganizationRow> = sqlx::query_as(
        "SELECT o.id, o.name, o.slug, o.status, o.is_system, om.role,
                (SELECT COUNT(*) FROM organization_memberships m
                  WHERE m.organization_id = o.id AND m.status = 'active') AS member_count,
                (SELECT COUNT(*) FROM organization_domains d
                  WHERE d.organization_id = o.id) AS domain_count,
                (SELECT COUNT(*) FROM organization_domains d
                  WHERE d.organization_id = o.id AND d.status = 'active') AS active_domain_count,
                (SELECT COUNT(*) FROM mailboxes mb
                  WHERE mb.organization_id = o.id AND mb.deleted_at IS NULL) AS mailbox_count
         FROM organizations o
         JOIN organization_memberships om ON om.organization_id = o.id
         WHERE om.user_id = $1 AND om.status = 'active'
         ORDER BY o.is_system DESC, o.created_at ASC",
    )
    .bind(auth.user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let selected: (Option<Uuid>, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT active_organization_id, active_mailbox_id, primary_mailbox_id FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let effective_mailbox_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT m.id
         FROM mailboxes m
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         WHERE m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'
           AND ($2::uuid IS NULL OR m.organization_id=$2)
         ORDER BY CASE WHEN m.id=$3 THEN 0 WHEN m.id=$4 THEN 1 ELSE 2 END, m.created_at ASC
         LIMIT 1",
    )
    .bind(auth.user_id)
    .bind(selected.0)
    .bind(selected.1)
    .bind(selected.2)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "organizations": rows.into_iter().map(|row| json!({
            "id": row.id,
            "name": row.name,
            "slug": row.slug,
            "status": row.status,
            "is_system": row.is_system,
            "role": row.role,
            "member_count": row.member_count,
            "domain_count": row.domain_count,
            "active_domain_count": row.active_domain_count,
            "mailbox_count": row.mailbox_count,
        })).collect::<Vec<_>>(),
        "active_organization_id": selected.0,
        "active_mailbox_id": effective_mailbox_id,
        "primary_mailbox_id": selected.2,
    })))
}

#[derive(Deserialize)]
pub struct CreateOrganizationIn {
    name: String,
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateOrganizationIn>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_business_creation(&state.db).await?;
    let verified: bool = sqlx::query_scalar(
        "SELECT email_verified_at IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if !verified {
        return Err(ApiError::forbidden("Verify your login email before creating a business"));
    }

    let name = normalize_name(&body.name)?;
    let slug = unique_slug(&state, &name).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let organization_id: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations(name, slug, created_by)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&name)
    .bind(&slug)
    .bind(auth.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO organization_subscriptions(organization_id,plan_code,status)
         VALUES ($1,'solo','suspended') ON CONFLICT (organization_id) DO NOTHING",
    )
    .bind(organization_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO organization_memberships(organization_id, user_id, role, status)
         VALUES ($1, $2, 'owner', 'active')",
    )
    .bind(organization_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        "UPDATE users SET active_organization_id = COALESCE(active_organization_id, $1), updated_at = now()
         WHERE id = $2",
    )
    .bind(organization_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(
        &state,
        Some(auth.user_id),
        "organization.create",
        json!({ "organization_id": organization_id, "name": name, "slug": slug }),
    )
    .await;

    Ok(Json(json!({
        "id": organization_id,
        "name": name,
        "slug": slug,
        "role": "owner",
        "status": "active",
    })))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let membership = tenancy::require_member(&state.db, auth.user_id, id).await?;
    let organization: Option<(String, String, String, bool)> = sqlx::query_as(
        "SELECT name, slug, status, is_system FROM organizations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (name, slug, status, is_system) = organization.ok_or_else(|| ApiError::not_found("Business not found"))?;

    let domains: Vec<(Uuid, String, String, bool, bool, String)> = sqlx::query_as(
        "SELECT id, domain::text, status, is_primary, is_system, last_error
         FROM organization_domains WHERE organization_id = $1 ORDER BY is_primary DESC, domain",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mailboxes: Vec<(Uuid, String, String, Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT id, address::text, display_name, user_id, status, sync_status
         FROM mailboxes WHERE organization_id = $1 AND deleted_at IS NULL ORDER BY address",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "id": id,
        "name": name,
        "slug": slug,
        "status": status,
        "is_system": is_system,
        "role": membership.role,
        "domains": domains.into_iter().map(|d| json!({
            "id": d.0, "domain": d.1, "status": d.2, "is_primary": d.3,
            "is_system": d.4, "last_error": d.5
        })).collect::<Vec<_>>(),
        "mailboxes": mailboxes.into_iter().map(|m| json!({
            "id": m.0, "address": m.1, "display_name": m.2, "user_id": m.3,
            "status": m.4, "sync_status": m.5
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct UpdateOrganizationIn {
    name: String,
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateOrganizationIn>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, id).await?;
    let is_system: bool = sqlx::query_scalar("SELECT is_system FROM organizations WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Business not found"))?;
    let name = normalize_name(&body.name)?;
    if is_system && name != "CrescentSphere" {
        return Err(ApiError::forbidden("The protected CrescentSphere business name cannot be changed here"));
    }
    sqlx::query("UPDATE organizations SET name = $1, updated_at = now() WHERE id = $2")
        .bind(&name)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(auth.user_id), "organization.update", json!({"organization_id": id, "name": name})).await;
    get(State(state), auth, Path(id)).await
}

pub async fn activate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_member(&state.db, auth.user_id, id).await?;
    let mailbox_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM mailboxes
         WHERE organization_id=$1 AND user_id=$2 AND deleted_at IS NULL AND status='active'
         ORDER BY is_primary_for_user DESC, created_at ASC
         LIMIT 1",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE users SET active_organization_id=$1,active_mailbox_id=$2,updated_at=now() WHERE id=$3")
        .bind(id)
        .bind(mailbox_id)
        .bind(auth.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok": true, "active_organization_id": id, "active_mailbox_id": mailbox_id})))
}

pub async fn members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_member(&state.db, auth.user_id, id).await?;
    let rows: Vec<(Uuid, String, String, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT u.id, u.email::text, u.display_name, om.role, om.status, om.joined_at
         FROM organization_memberships om
         JOIN users u ON u.id = om.user_id
         WHERE om.organization_id = $1
         ORDER BY CASE om.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'billing' THEN 2 ELSE 3 END,
                  lower(u.email::text)",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"members": rows.into_iter().map(|r| json!({
        "user_id": r.0, "email": r.1, "display_name": r.2, "role": r.3,
        "status": r.4, "joined_at": r.5
    })).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct InviteIn {
    email: String,
    #[serde(default = "default_member_role")]
    role: String,
}
fn default_member_role() -> String { "member".to_string() }

pub async fn invitations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, id).await?;
    let rows: Vec<(Uuid, String, String, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, email::text, role, status, expires_at, created_at
         FROM organization_invitations
         WHERE organization_id = $1
         ORDER BY created_at DESC LIMIT 200",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"invitations": rows.into_iter().map(|r| json!({
        "id": r.0, "email": r.1, "role": r.2, "status": r.3,
        "expires_at": r.4, "created_at": r.5
    })).collect::<Vec<_>>() })))
}

pub async fn invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<InviteIn>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, id).await?;
    crate::services::entitlements::require_capacity(&state, id, crate::services::entitlements::CapacityKind::Seat, 1).await?;
    let target = body.email.trim().to_lowercase();
    if !valid_email(&target) {
        return Err(ApiError::bad_request("Invalid invitation email"));
    }
    if !matches!(body.role.as_str(), "owner" | "admin" | "billing" | "member") {
        return Err(ApiError::bad_request("Invalid business role"));
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM organization_memberships om
           JOIN users u ON u.id = om.user_id
           WHERE om.organization_id = $1 AND lower(u.email::text) = lower($2) AND om.status = 'active'
         )",
    )
    .bind(id)
    .bind(&target)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if exists {
        return Err(ApiError::conflict("This person is already a member"));
    }

    let org_name: String = sqlx::query_scalar("SELECT name FROM organizations WHERE id = $1")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let token = email::random_token();
    let hash = token_hash(&token);
    let invitation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO organization_invitations(
             organization_id, email, role, token_hash, invited_by, expires_at
         ) VALUES ($1, $2, $3, $4, $5, now() + ($6 * interval '1 day'))
         ON CONFLICT (organization_id, lower(email::text)) WHERE status = 'pending'
         DO UPDATE SET role = EXCLUDED.role, token_hash = EXCLUDED.token_hash,
                       invited_by = EXCLUDED.invited_by, expires_at = EXCLUDED.expires_at,
                       updated_at = now()
         RETURNING id",
    )
    .bind(id)
    .bind(&target)
    .bind(&body.role)
    .bind(&hash)
    .bind(auth.user_id)
    .bind(INVITE_TTL_DAYS)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let link = format!("{}/business-invite?token={}", state.public_origin, token);
    if let Err(err) = email::send_business_invitation(&state, &target, &org_name, &link).await {
        let _ = sqlx::query(
            "UPDATE organization_invitations SET status='revoked', revoked_at=now(), updated_at=now() WHERE id=$1",
        )
        .bind(invitation_id)
        .execute(&state.db)
        .await;
        return Err(err);
    }
    audit::record(&state, Some(auth.user_id), "organization.invite", json!({
        "organization_id": id, "invitation_id": invitation_id, "email": target, "role": body.role
    })).await;

    let mut value = json!({"ok": true, "id": invitation_id, "expires_in_days": INVITE_TTL_DAYS});
    if state.return_token_links {
        value["dev"] = json!({"invite_link": link});
    }
    Ok(Json(value))
}

pub async fn revoke_invitation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, invitation_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    let changed = sqlx::query(
        "UPDATE organization_invitations
         SET status='revoked', revoked_at=now(), updated_at=now()
         WHERE id=$1 AND organization_id=$2 AND status='pending'",
    )
    .bind(invitation_id)
    .bind(organization_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .rows_affected();
    if changed == 0 {
        return Err(ApiError::not_found("Pending invitation not found"));
    }
    audit::record(&state, Some(auth.user_id), "organization.invite.revoke", json!({
        "organization_id": organization_id, "invitation_id": invitation_id
    })).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct AcceptInviteIn {
    token: String,
}

pub async fn accept_invitation(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AcceptInviteIn>,
) -> Result<Json<Value>, ApiError> {
    let hash = token_hash(body.token.trim());
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let invitation: Option<(Uuid, Uuid, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, organization_id, email::text, role, expires_at
         FROM organization_invitations
         WHERE token_hash=$1 AND status='pending'
         FOR UPDATE",
    )
    .bind(hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (invitation_id, organization_id, target_email, role, expires_at) = invitation
        .ok_or_else(|| ApiError::bad_request("This business invitation is invalid or has already been used"))?;
    if expires_at <= chrono::Utc::now() {
        sqlx::query("UPDATE organization_invitations SET status='expired', updated_at=now() WHERE id=$1")
            .bind(invitation_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(ApiError::bad_request("This business invitation has expired"));
    }
    if !target_email.eq_ignore_ascii_case(&auth.email) {
        return Err(ApiError::forbidden("Sign in with the email address that received this invitation"));
    }

    // Mark the pending invitation consumed before activating membership so
    // the database seat-capacity guard counts this person exactly once.
    sqlx::query(
        "UPDATE organization_invitations
         SET status='accepted', accepted_by=$1, accepted_at=now(), updated_at=now()
         WHERE id=$2",
    )
    .bind(auth.user_id)
    .bind(invitation_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO organization_memberships(organization_id,user_id,role,status)
         VALUES($1,$2,$3,'active')
         ON CONFLICT(organization_id,user_id) DO UPDATE
           SET role=EXCLUDED.role,status='active',updated_at=now()",
    )
    .bind(organization_id)
    .bind(auth.user_id)
    .bind(&role)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query(
        "UPDATE users SET active_organization_id = COALESCE(active_organization_id, $1), updated_at=now()
         WHERE id=$2",
    )
    .bind(organization_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    audit::record(&state, Some(auth.user_id), "organization.invite.accept", json!({
        "organization_id": organization_id, "invitation_id": invitation_id, "role": role
    })).await;
    Ok(Json(json!({"ok": true, "organization_id": organization_id, "role": role})))
}
