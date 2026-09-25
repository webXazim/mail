use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{email, tenancy};
use crate::state::AppState;

const INVITE_TTL_DAYS: i64 = 7;

fn token_hash(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    format!("{:x}", h.finalize())
}

fn valid_local(value: &str) -> bool {
    let v = value.trim();
    !v.is_empty()
        && v.len() <= 64
        && !v.starts_with('.')
        && !v.ends_with('.')
        && !v.contains("..")
        && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn valid_role(value: &str) -> bool {
    matches!(value, "owner" | "admin" | "billing" | "member")
}

#[derive(Deserialize)]
pub struct CreateMailboxIn {
    pub domain_id: Uuid,
    pub local_part: String,
    #[serde(default)]
    pub display_name: String,
    pub member_user_id: Option<Uuid>,
    pub invite_email: Option<String>,
    #[serde(default = "default_role")]
    pub role: String,
    /// Optional initial reservation from the organization's pooled storage.
    /// Omitted means the plan's default per-mailbox allocation.
    pub quota_bytes: Option<i64>,
}
fn default_role() -> String { "member".to_string() }

#[derive(sqlx::FromRow)]
struct MailboxAdminRow {
    id: Uuid,
    domain_id: Uuid,
    address: String,
    display_name: String,
    user_id: Option<Uuid>,
    invited_email: Option<String>,
    status: String,
    sync_status: String,
    quota_bytes: i64,
    quota_override_bytes: Option<i64>,
    provider_account_id: Option<String>,
    sync_error: String,
    cached_used_bytes: Option<i64>,
    cached_provider_quota_bytes: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let membership=tenancy::require_member(&state.db, auth.user_id, organization_id).await?;
    let can_manage_storage=matches!(membership.role.as_str(),"owner"|"admin");
    let rows: Vec<MailboxAdminRow> = sqlx::query_as(
        "SELECT m.id,m.domain_id,m.address::text AS address,m.display_name,m.user_id,m.invited_email::text AS invited_email,
                m.status,m.sync_status,m.quota_bytes,m.quota_override_bytes,m.provider_account_id,m.sync_error,
                r.quota_used AS cached_used_bytes,r.provider_quota_total AS cached_provider_quota_bytes
         FROM mailboxes m
         LEFT JOIN realtime_mailbox_state r ON r.mailbox_id=m.id
         WHERE m.organization_id=$1 AND m.deleted_at IS NULL ORDER BY lower(m.address::text)",
    ).bind(organization_id).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;

    let accounts: Vec<String>=rows.iter().filter(|row| can_manage_storage || row.user_id==Some(auth.user_id))
        .filter_map(|row| row.provider_account_id.clone().filter(|id| !id.trim().is_empty())).collect();
    let live_quotas=state.stalwart.account_quotas(&accounts).await.unwrap_or_default();
    let ent=crate::services::entitlements::for_organization(&state,organization_id).await?;
    let allocated_bytes: i64=rows.iter().map(|row| row.quota_bytes.max(0)).sum();
    let mut used_total=0i64;
    let mailboxes=rows.into_iter().map(|row| {
        let visible_usage=can_manage_storage || row.user_id==Some(auth.user_id);
        let live=row.provider_account_id.as_ref().and_then(|id| live_quotas.get(id));
        let used=if visible_usage && row.provider_account_id.is_some() {
            live.map(|q| q.0.min(i64::MAX as u64) as i64).or(row.cached_used_bytes).map(|value| value.max(0))
        } else { None };
        if let Some(used) = used { used_total=used_total.saturating_add(used); }
        let provider_total=if visible_usage {
            live.map(|q| q.1.min(i64::MAX as u64) as i64).or(row.cached_provider_quota_bytes)
        } else { None };
        json!({
            "id":row.id,"domain_id":row.domain_id,"address":row.address,"display_name":row.display_name,"user_id":row.user_id,
            "invited_email":row.invited_email,"status":row.status,"sync_status":row.sync_status,
            "quota_bytes":if visible_usage { Some(row.quota_bytes) } else { None::<i64> },
            "quota_override_bytes":if visible_usage { row.quota_override_bytes } else { None::<i64> },
            "quota_source":if visible_usage { Some(if row.quota_override_bytes.is_some(){"custom"}else{"default"}) } else { None::<&str> },
            "used_bytes":used,
            "provider_quota_bytes":provider_total,"quota_in_sync":provider_total.map(|value| value==row.quota_bytes),
            "storage_pct":if row.quota_bytes>0 { used.map(|used| ((used as f64/row.quota_bytes as f64)*100.0).clamp(0.0,100.0).round() as i64) } else { None },
            "sync_error":if row.sync_error.contains("not authorized to grant permissions") {
                "Mail service authorization needs operator attention. Please contact support; retry setup after the provider credential is corrected."
            } else if row.sync_error.contains("Mailbox has not been provisioned yet") {
                "Storage will sync after mailbox setup completes."
            } else if row.sync_error.is_empty() { "" } else {
                "Mail service could not complete this mailbox operation. Retry setup or contact support."
            }
        })
    }).collect::<Vec<_>>();
    let pool=ent.storage_pool_bytes.min(i64::MAX as u64) as i64;
    Ok(Json(json!({
        "mailboxes":mailboxes,
        "storage":if can_manage_storage { Some(json!({
            "pool_bytes":pool,
            "allocated_bytes":allocated_bytes,
            "unallocated_bytes":pool.saturating_sub(allocated_bytes),
            "used_bytes":used_total,
            "default_mailbox_bytes":ent.quota_bytes,
            "can_manage":true
        })) } else { None::<Value> }
    })))
}

pub async fn invitations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    let rows: Vec<(Uuid, Uuid, String, String, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT mi.id,mi.mailbox_id,mi.email::text,mi.role,mi.status,mi.expires_at,mi.created_at
         FROM mailbox_invitations mi WHERE mi.organization_id=$1 ORDER BY mi.created_at DESC LIMIT 200",
    ).bind(organization_id).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"invitations": rows.into_iter().map(|r| json!({
        "id":r.0,"mailbox_id":r.1,"email":r.2,"role":r.3,"status":r.4,"expires_at":r.5,"created_at":r.6
    })).collect::<Vec<_>>() })))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(organization_id): Path<Uuid>,
    Json(body): Json<CreateMailboxIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    crate::services::platform_control::require_mailbox_provisioning(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    crate::services::entitlements::require_capacity(&state, organization_id, crate::services::entitlements::CapacityKind::Mailbox, 1).await?;
    let local = body.local_part.trim().to_ascii_lowercase();
    if !valid_local(&local) { return Err(ApiError::bad_request("Invalid mailbox local part")); }
    if !valid_role(&body.role) { return Err(ApiError::bad_request("Invalid business role")); }
    if body.member_user_id.is_some() == body.invite_email.as_ref().is_some_and(|v| !v.trim().is_empty()) {
        return Err(ApiError::bad_request("Choose exactly one existing member or invitation email"));
    }
    let domain: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT domain::text,status,provider_domain_id FROM organization_domains
         WHERE id=$1 AND organization_id=$2",
    ).bind(body.domain_id).bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (domain_name, domain_status, provider_domain_id) = domain.ok_or_else(|| ApiError::not_found("Business domain not found"))?;
    if domain_status != "active" || provider_domain_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ApiError::conflict("Mailboxes can only be created on an active mail domain"));
    }
    let address = format!("{local}@{domain_name}");
    let mailbox_id = Uuid::new_v4();
    let marker = state.stalwart.ownership_marker("mailbox", &organization_id.to_string(), &mailbox_id.to_string());
    let display_name = body.display_name.trim().chars().take(120).collect::<String>();
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let mailbox_quota=crate::services::entitlements::mailbox_allocation_tx(&mut tx,organization_id,body.quota_bytes).await?;

    let assigned_user = body.member_user_id;
    if let Some(user_id) = assigned_user {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 AND status='active')",
        ).bind(organization_id).bind(user_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        if !active { return Err(ApiError::bad_request("Selected user is not an active business member")); }
    }

    sqlx::query(
        "INSERT INTO mailboxes(id,organization_id,domain_id,user_id,address,local_part,display_name,status,
           is_primary_for_user,provider_marker,sync_status,quota_bytes,quota_override_bytes,invited_email)
         VALUES($1,$2,$3,$4,$5,$6,$7,'pending',FALSE,$8,'pending',$9,$10,$11)",
    ).bind(mailbox_id).bind(organization_id).bind(body.domain_id).bind(assigned_user)
     .bind(&address).bind(&local).bind(&display_name).bind(&marker).bind(mailbox_quota)
     .bind(body.quota_bytes.map(|_| mailbox_quota))
     .bind(body.invite_email.as_ref().map(|v| v.trim().to_ascii_lowercase()))
     .execute(&mut *tx).await.map_err(|e| {
        if let sqlx::Error::Database(db)=&e { if db.code().as_deref()==Some("23505") { return ApiError::conflict("That business address is already in use"); } }
        ApiError::internal(e.to_string())
     })?;

    let mut invitation: Option<(Uuid,String,String)> = None;
    if let Some(user_id) = assigned_user {
        let has_primary: bool = sqlx::query_scalar("SELECT primary_mailbox_id IS NOT NULL FROM users WHERE id=$1")
            .bind(user_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        if !has_primary {
            sqlx::query("UPDATE users SET primary_mailbox_id=$1,active_mailbox_id=$1,active_organization_id=$2,updated_at=now() WHERE id=$3")
                .bind(mailbox_id).bind(organization_id).bind(user_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
            sqlx::query("UPDATE mailboxes SET is_primary_for_user=TRUE WHERE id=$1")
                .bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        }
        let provider_secret = email::random_token();
        state.provisioning.enqueue_mailbox_ensure_tx(&mut tx, mailbox_id, user_id, &provider_secret).await?;
    } else if let Some(invite_email) = body.invite_email.as_ref().map(|v| v.trim().to_ascii_lowercase()) {
        if !invite_email.contains('@') { return Err(ApiError::bad_request("Invalid invitation email")); }
        let token = email::random_token();
        let hash = token_hash(&token);
        let invitation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO mailbox_invitations(id,organization_id,mailbox_id,email,role,token_hash,invited_by,expires_at)
             VALUES($1,$2,$3,$4,$5,$6,$7,now()+($8*interval '1 day'))",
        ).bind(invitation_id).bind(organization_id).bind(mailbox_id).bind(&invite_email).bind(&body.role)
         .bind(hash).bind(auth.user_id).bind(INVITE_TTL_DAYS).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        invitation=Some((invitation_id,token,invite_email));
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some((invitation_id, token, target)) = invitation {
        let org_name: String = sqlx::query_scalar("SELECT name FROM organizations WHERE id=$1")
            .bind(organization_id).fetch_one(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
        let link = format!("{}/business-invite?mailbox_token={}", state.public_origin, token);
        if let Err(error)=email::send_mailbox_invitation(&state,&target,&org_name,&address,&link).await {
            let _=sqlx::query("UPDATE mailbox_invitations SET status='revoked',revoked_at=now(),updated_at=now() WHERE id=$1")
                .bind(invitation_id).execute(&state.db).await;
            let _=sqlx::query("UPDATE mailboxes SET status='error',sync_status='error',sync_error='Invitation delivery failed',updated_at=now() WHERE id=$1")
                .bind(mailbox_id).execute(&state.db).await;
            return Err(error);
        }
    }
    audit::record(&state,Some(auth.user_id),"business.mailbox.create",json!({"organization_id":organization_id,"mailbox_id":mailbox_id,"address":address,"assigned_user":assigned_user})).await;
    Ok((axum::http::StatusCode::CREATED,Json(json!({"id":mailbox_id,"address":address,"status":"pending","user_id":assigned_user,"quota_bytes":mailbox_quota}))))
}

/// Requeue a failed provider creation without deleting the customer's address
/// or changing its reserved storage allocation. The provider credential stays
/// internal; external mail clients use one-time app passwords after activation.
pub async fn retry_provisioning(
    State(state): State<AppState>, auth: AuthUser,
    Path((organization_id, mailbox_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    crate::services::platform_control::require_mailbox_provisioning(&state.db).await?;
    tenancy::require_admin(&state.db, auth.user_id, organization_id).await?;
    state.rate.check_burst(&format!("mailbox-retry:{organization_id}:{mailbox_id}"), 5, 3600).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let target: Option<(Option<Uuid>, String, Option<String>, String)> = sqlx::query_as(
        "SELECT m.user_id,m.status,m.provider_account_id,d.status
         FROM mailboxes m JOIN organization_domains d ON d.id=m.domain_id
         WHERE m.id=$1 AND m.organization_id=$2 AND m.deleted_at IS NULL FOR UPDATE OF m"
    ).bind(mailbox_id).bind(organization_id).fetch_optional(&mut *tx).await
     .map_err(|e| ApiError::internal(e.to_string()))?;
    let (user_id, status, provider_account_id, domain_status) = target.ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    let user_id = user_id.ok_or_else(|| ApiError::conflict("The invited member must accept the mailbox before it can be provisioned"))?;
    if domain_status != "active" { return Err(ApiError::conflict("The domain must be active before retrying mailbox setup")); }
    if provider_account_id.as_deref().is_some_and(|value| !value.is_empty()) || !matches!(status.as_str(), "error" | "pending") {
        return Err(ApiError::conflict("This mailbox is already provisioned or cannot be retried"));
    }
    sqlx::query("UPDATE mailboxes SET status='pending',sync_status='pending',sync_error='',updated_at=now() WHERE id=$1")
        .bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let provider_secret = email::random_token();
    state.provisioning.enqueue_mailbox_ensure_tx(&mut tx, mailbox_id, user_id, &provider_secret).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(auth.user_id), "business.mailbox.retry", json!({"organization_id":organization_id,"mailbox_id":mailbox_id})).await;
    Ok(Json(json!({"ok":true,"status":"pending"})))
}

pub async fn activate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((organization_id, mailbox_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let mailbox = tenancy::set_active_mailbox(&state.db, auth.user_id, organization_id, mailbox_id).await?;
    audit::record(&state, Some(auth.user_id), "business.mailbox.activate", json!({
        "organization_id": organization_id, "mailbox_id": mailbox_id, "address": mailbox.address
    })).await;
    Ok(Json(json!({
        "ok": true,
        "active_organization_id": organization_id,
        "active_mailbox_id": mailbox_id,
        "address": mailbox.address
    })))
}

#[derive(Deserialize)]
pub struct UpdateMailboxIn { pub status: String, #[serde(default)] pub display_name: String }

pub async fn update(
    State(state): State<AppState>, auth: AuthUser,
    Path((organization_id,mailbox_id)): Path<(Uuid,Uuid)>, Json(body): Json<UpdateMailboxIn>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    if !matches!(body.status.as_str(),"active"|"suspended") { return Err(ApiError::bad_request("Mailbox status must be active or suspended")); }
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let changed=sqlx::query(
        "UPDATE mailboxes SET status=$3,display_name=CASE WHEN $4='' THEN display_name ELSE $4 END,
         suspended_at=CASE WHEN $3='suspended' THEN now() ELSE NULL END,updated_at=now()
         WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL AND user_id IS NOT NULL"
    ).bind(mailbox_id).bind(organization_id).bind(&body.status).bind(body.display_name.trim())
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?.rows_affected();
    if changed==0 { return Err(ApiError::not_found("Assigned mailbox not found")); }
    if body.status == "suspended" {
        sqlx::query("UPDATE users SET active_mailbox_id=NULL,updated_at=now() WHERE active_mailbox_id=$1")
            .bind(mailbox_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    state.provisioning.enqueue_mailbox_access_tx(&mut tx,mailbox_id).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"business.mailbox.status",json!({"organization_id":organization_id,"mailbox_id":mailbox_id,"status":body.status})).await;
    Ok(Json(json!({"ok":true,"status":body.status})))
}

#[derive(Deserialize)]
pub struct UpdateMailboxStorageIn {
    pub quota_bytes: Option<i64>,
    #[serde(default)]
    pub reset_to_default: bool,
}

pub async fn update_storage(
    State(state): State<AppState>, auth: AuthUser,
    Path((organization_id,mailbox_id)): Path<(Uuid,Uuid)>, Json(body): Json<UpdateMailboxStorageIn>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    if body.reset_to_default == body.quota_bytes.is_some() {
        return Err(ApiError::bad_request("Provide quota_bytes or reset_to_default, but not both"));
    }
    let ent=crate::services::entitlements::for_organization(&state,organization_id).await?;
    let default_quota=ent.quota_bytes.min(i64::MAX as u64) as i64;
    let target=if body.reset_to_default { default_quota } else { body.quota_bytes.unwrap_or(0) };
    if target < 1_048_576 { return Err(ApiError::bad_request("Mailbox storage allocation must be at least 1 MiB")); }
    if target as u64 > ent.storage_pool_bytes { return Err(ApiError::bad_request("A mailbox allocation cannot exceed the business storage pool")); }

    let mailbox: Option<(String,Option<String>,i64,Option<String>,String,String)> = sqlx::query_as(
        "SELECT m.address::text,m.provider_account_id,m.quota_bytes,d.provider_domain_id,m.local_part,m.provider_marker
         FROM mailboxes m JOIN organization_domains d ON d.id=m.domain_id
         WHERE m.id=$1 AND m.organization_id=$2 AND m.deleted_at IS NULL AND m.status <> 'deleting'"
    ).bind(mailbox_id).bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (address,provider_account_id,current_quota,provider_domain_id,local_part,provider_marker)=mailbox.ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    if !matches!(ent.subscription_status.as_str(), "active" | "trial" | "past_due") {
        return Err(ApiError::forbidden("Renew the business plan before changing mailbox storage"));
    }
    if ent.subscription_status == "past_due" && target > current_quota {
        return Err(ApiError::forbidden("Renew the business plan before increasing mailbox storage"));
    }

    let cached_used: Option<i64>=sqlx::query_scalar("SELECT quota_used FROM realtime_mailbox_state WHERE mailbox_id=$1")
        .bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?.flatten();
    let live_used=if let Some(account)=provider_account_id.as_deref().filter(|value| !value.trim().is_empty()) {
        match state.stalwart.account_quota(account).await {
            Ok(Some((used,_))) => Some(used.min(i64::MAX as u64) as i64),
            Ok(None) => None,
            Err(_) => None,
        }
    } else { None };
    let mut known_used=live_used.or(cached_used).map(|value| value.max(0));
    if target < current_quota && known_used.is_none() && provider_account_id.is_none() {
        // A pending mailbox may have been created by Stalwart just before a
        // lost response. Check its ownership binding before assuming zero use.
        let domain_id=provider_domain_id.as_deref().ok_or_else(|| ApiError::conflict("Mail provider domain is unavailable"))?;
        let found=state.stalwart.find_customer_account(domain_id,&local_part,&provider_marker).await
            .map_err(|error| ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"mail_provider",error.public_message()))?;
        known_used=if let Some(account)=found {
            state.stalwart.account_quota(&account).await
                .map_err(|error| ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"mail_provider",error.public_message()))?
                .map(|(used,_)| used.min(i64::MAX as u64) as i64)
        } else { Some(0) };
    }
    if target < current_quota && known_used.is_none() {
        return Err(ApiError::conflict("Current mailbox usage could not be verified. Retry after provider usage is available before lowering this allocation"));
    }
    let used=known_used.unwrap_or(0);
    if target < used {
        return Err(ApiError::conflict(format!("This mailbox currently uses {used} bytes. Its allocation cannot be lowered below current usage")));
    }

    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let pool: i64=sqlx::query_scalar(
        "SELECT COALESCE(s.storage_pool_override_bytes,COALESCE(pv.storage_pool_bytes,p.storage_pool_bytes)::bigint + COALESCE(pv.mailbox_bytes,p.mailbox_bytes)::bigint*GREATEST(s.purchased_mailbox_count-COALESCE(pv.mailbox_limit,p.mailbox_limit),0)::bigint)::bigint
         FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
         WHERE s.organization_id=$1 FOR UPDATE OF s"
    ).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?
     .ok_or_else(|| ApiError::forbidden("Business subscription is not available"))?;
    let allocated_other: i64=sqlx::query_scalar(
        "SELECT COALESCE(SUM(quota_bytes),0)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND id<>$2"
    ).bind(organization_id).bind(mailbox_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if allocated_other.saturating_add(target)>pool {
        return Err(ApiError::conflict(format!("Only {} bytes remain available in the business storage pool",pool.saturating_sub(allocated_other))));
    }
    sqlx::query(
        "UPDATE mailboxes SET quota_bytes=$3,quota_override_bytes=$4,quota_updated_by=$5,quota_updated_at=now(),updated_at=now()
         WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL"
    ).bind(mailbox_id).bind(organization_id).bind(target)
     .bind(if body.reset_to_default { None::<i64> } else { Some(target) }).bind(auth.user_id)
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    // A mailbox awaiting provider creation has a reserved allocation, not a
    // provider quota yet. Its ensure job reads the current allocation at run time.
    if provider_account_id.as_deref().is_some_and(|value| !value.is_empty()) {
        state.provisioning.enqueue_mailbox_quota_tx(&mut tx,mailbox_id,target).await?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"business.mailbox.storage",json!({
        "organization_id":organization_id,"mailbox_id":mailbox_id,"address":address,
        "previous_quota_bytes":current_quota,"quota_bytes":target,"reset_to_default":body.reset_to_default
    })).await;
    Ok(Json(json!({"ok":true,"mailbox_id":mailbox_id,"quota_bytes":target,"quota_source":if body.reset_to_default{"default"}else{"custom"}})))
}

/// Spread only the currently unreserved organization storage across existing
/// mailboxes. This operation never lowers an allocation, so it cannot place a
/// mailbox below its actual provider usage. Individual allocations remain
/// editable afterwards.
pub async fn distribute_available_storage(
    State(state): State<AppState>, auth: AuthUser, Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let subscription: Option<(i64,String)>=sqlx::query_as(
        "SELECT COALESCE(s.storage_pool_override_bytes,
                         COALESCE(pv.storage_pool_bytes,p.storage_pool_bytes)::bigint+COALESCE(pv.mailbox_bytes,p.mailbox_bytes)::bigint*GREATEST(s.purchased_mailbox_count-COALESCE(pv.mailbox_limit,p.mailbox_limit),0)::bigint)::bigint,
                CASE
                  WHEN s.status IN ('active','trial') AND s.current_period_end IS NOT NULL AND s.current_period_end<=now() THEN 'past_due'
                  WHEN s.status='past_due' AND s.renewal_grace_end IS NOT NULL AND s.renewal_grace_end<=now() THEN 'suspended'
                  ELSE s.status
                END
         FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
         WHERE s.organization_id=$1 FOR UPDATE OF s"
    ).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (pool,status)=subscription.ok_or_else(|| ApiError::forbidden("Business subscription is not available"))?;
    if !matches!(status.as_str(),"active"|"trial") {
        return Err(ApiError::forbidden("Renew the business plan before increasing mailbox storage"));
    }

    let allocated: i64=sqlx::query_scalar(
        "SELECT COALESCE(SUM(quota_bytes),0)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL"
    ).bind(organization_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let available=pool.saturating_sub(allocated);
    if available<=0 {
        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok(Json(json!({"ok":true,"distributed_bytes":0,"pool_bytes":pool,"allocated_bytes":allocated,"unallocated_bytes":0})));
    }

    let mailboxes: Vec<(Uuid,i64,Option<String>)>=sqlx::query_as(
        "SELECT id,quota_bytes,provider_account_id FROM mailboxes
         WHERE organization_id=$1 AND deleted_at IS NULL AND status<>'deleting'
         ORDER BY created_at,id FOR UPDATE"
    ).bind(organization_id).fetch_all(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if mailboxes.is_empty() {
        return Err(ApiError::conflict("Create a mailbox before distributing pooled storage"));
    }
    let count=mailboxes.len() as i64;
    let share=available/count;
    let remainder=available%count;
    for (index,(mailbox_id,current_quota,provider_account_id)) in mailboxes.into_iter().enumerate() {
        let target=current_quota.saturating_add(share).saturating_add(if (index as i64)<remainder {1}else{0});
        sqlx::query(
            "UPDATE mailboxes SET quota_bytes=$2,quota_override_bytes=$2,quota_updated_by=$3,quota_updated_at=now(),updated_at=now()
             WHERE id=$1"
        ).bind(mailbox_id).bind(target).bind(auth.user_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        if provider_account_id.as_deref().is_some_and(|value| !value.trim().is_empty()) {
            state.provisioning.enqueue_mailbox_quota_tx(&mut tx,mailbox_id,target).await?;
        }
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"business.storage.distribute",json!({
        "organization_id":organization_id,"distributed_bytes":available,"pool_bytes":pool
    })).await;
    Ok(Json(json!({"ok":true,"distributed_bytes":available,"pool_bytes":pool,"allocated_bytes":pool,"unallocated_bytes":0})))
}

pub async fn delete(
    State(state): State<AppState>, auth: AuthUser,
    Path((organization_id,mailbox_id)): Path<(Uuid,Uuid)>,
) -> Result<Json<Value>, ApiError> {
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(Option<Uuid>,)> = sqlx::query_as(
        "SELECT user_id FROM mailboxes WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL FOR UPDATE",
    ).bind(mailbox_id).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (user_id,)=row.ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
    let used_by_address: bool=sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM business_address_members WHERE mailbox_id=$1)",
    ).bind(mailbox_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if used_by_address { return Err(ApiError::conflict("Remove this mailbox from aliases/groups before deleting it")); }
    sqlx::query("UPDATE mailbox_invitations SET status='revoked',revoked_at=now(),updated_at=now() WHERE mailbox_id=$1 AND status='pending'")
        .bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE mailboxes SET status='deleting',updated_at=now() WHERE id=$1")
        .bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(user_id)=user_id {
        sqlx::query(
            "UPDATE users SET
               primary_mailbox_id=CASE WHEN primary_mailbox_id=$2 THEN NULL ELSE primary_mailbox_id END,
               active_mailbox_id=CASE WHEN active_mailbox_id=$2 THEN NULL ELSE active_mailbox_id END,
               updated_at=now()
             WHERE id=$1 AND (primary_mailbox_id=$2 OR active_mailbox_id=$2)",
        )
        .bind(user_id)
        .bind(mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    state.provisioning.enqueue_mailbox_delete_tx(&mut tx,mailbox_id).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"business.mailbox.delete",json!({"organization_id":organization_id,"mailbox_id":mailbox_id})).await;
    Ok(Json(json!({"ok":true,"status":"deleting"})))
}

pub async fn revoke_invitation(
    State(state): State<AppState>,auth: AuthUser,
    Path((organization_id,invitation_id)): Path<(Uuid,Uuid)>,
) -> Result<Json<Value>,ApiError> {
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let mailbox_id: Option<Uuid>=sqlx::query_scalar(
        "UPDATE mailbox_invitations SET status='revoked',revoked_at=now(),updated_at=now()
         WHERE id=$1 AND organization_id=$2 AND status='pending' RETURNING mailbox_id",
    ).bind(invitation_id).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let mailbox_id=mailbox_id.ok_or_else(|| ApiError::not_found("Pending mailbox invitation not found"))?;
    sqlx::query("UPDATE mailboxes SET status='error',sync_status='none',sync_error='Invitation revoked',updated_at=now() WHERE id=$1 AND user_id IS NULL")
        .bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub struct AcceptMailboxInviteIn { pub token: String }

pub async fn accept_invitation(
    State(state): State<AppState>,auth: AuthUser,Json(body): Json<AcceptMailboxInviteIn>,
) -> Result<Json<Value>,ApiError> {
    crate::services::platform_control::require_mailbox_provisioning(&state.db).await?;
    let hash=token_hash(body.token.trim());
    let mut tx=state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let invitation: Option<(Uuid,Uuid,Uuid,String,String,chrono::DateTime<chrono::Utc>)>=sqlx::query_as(
        "SELECT id,organization_id,mailbox_id,email::text,role,expires_at FROM mailbox_invitations
         WHERE token_hash=$1 AND status='pending' FOR UPDATE",
    ).bind(hash).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (invitation_id,organization_id,mailbox_id,target_email,role,expires_at)=invitation.ok_or_else(|| ApiError::bad_request("This mailbox invitation is invalid or already used"))?;
    if expires_at<=chrono::Utc::now() {
        sqlx::query("UPDATE mailbox_invitations SET status='expired',updated_at=now() WHERE id=$1").bind(invitation_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
        return Err(ApiError::bad_request("This mailbox invitation has expired"));
    }
    if !target_email.eq_ignore_ascii_case(&auth.email) { return Err(ApiError::forbidden("Sign in with the email address that received this mailbox invitation")); }
    let mailbox_free: bool=sqlx::query_scalar("SELECT user_id IS NULL AND status IN ('pending','error') FROM mailboxes WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL FOR UPDATE")
        .bind(mailbox_id).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?.unwrap_or(false);
    if !mailbox_free { return Err(ApiError::conflict("This mailbox has already been assigned")); }
    sqlx::query(
        "INSERT INTO organization_memberships(organization_id,user_id,role,status) VALUES($1,$2,$3,'active')
         ON CONFLICT(organization_id,user_id) DO UPDATE SET role=EXCLUDED.role,status='active',updated_at=now()",
    ).bind(organization_id).bind(auth.user_id).bind(&role).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let has_primary: bool=sqlx::query_scalar("SELECT primary_mailbox_id IS NOT NULL FROM users WHERE id=$1")
        .bind(auth.user_id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE mailboxes SET user_id=$2,status='pending',sync_status='pending',sync_error='',updated_at=now() WHERE id=$1")
        .bind(mailbox_id).bind(auth.user_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if !has_primary {
        sqlx::query("UPDATE users SET primary_mailbox_id=$1,active_mailbox_id=$1,active_organization_id=$2,updated_at=now() WHERE id=$3")
            .bind(mailbox_id).bind(organization_id).bind(auth.user_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET is_primary_for_user=TRUE WHERE id=$1").bind(mailbox_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    }
    sqlx::query("UPDATE mailbox_invitations SET status='accepted',accepted_by=$1,accepted_at=now(),updated_at=now() WHERE id=$2")
        .bind(auth.user_id).bind(invitation_id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let provider_secret=email::random_token();
    state.provisioning.enqueue_mailbox_ensure_tx(&mut tx,mailbox_id,auth.user_id,&provider_secret).await?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"business.mailbox.invite.accept",json!({"organization_id":organization_id,"mailbox_id":mailbox_id,"invitation_id":invitation_id})).await;
    Ok(Json(json!({"ok":true,"organization_id":organization_id,"mailbox_id":mailbox_id})))
}
