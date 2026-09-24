use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AdminUser;
use crate::services::{domain_onboarding, platform_control};
use crate::state::AppState;

const PAGE_DEFAULT: i64 = 50;
const PAGE_MAX: i64 = 200;

fn page(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX), offset.unwrap_or(0).max(0))
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    q: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct BusinessRow {
    id: Uuid,
    name: String,
    slug: String,
    status: String,
    status_reason: String,
    status_changed_at: Option<DateTime<Utc>>,
    owner_email: Option<String>,
    member_count: i64,
    domain_count: i64,
    mailbox_count: i64,
    active_mailbox_count: i64,
    plan_code: Option<String>,
    plan_name: Option<String>,
    subscription_status: Option<String>,
    purchased_mailbox_count: Option<i32>,
    current_period_end: Option<DateTime<Utc>>,
    assignment_source: Option<String>,
    created_at: DateTime<Utc>,
}

pub async fn businesses(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let q = query.q.unwrap_or_default().trim().chars().take(200).collect::<String>();
    let status = query.status.filter(|value| !value.trim().is_empty());
    if status.as_deref().is_some_and(|value| !matches!(value, "active" | "suspended" | "closed")) {
        return Err(ApiError::bad_request("Business status filter is invalid"));
    }
    let (limit, offset) = page(query.limit, query.offset);
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM organizations o
         WHERE o.is_system=FALSE
           AND ($1::text='' OR o.name ILIKE '%'||$1||'%' OR o.slug ILIKE '%'||$1||'%'
             OR EXISTS(SELECT 1 FROM organization_memberships om JOIN users u ON u.id=om.user_id
                       WHERE om.organization_id=o.id AND u.email::text ILIKE '%'||$1||'%'))
           AND ($2::text IS NULL OR o.status=$2)"
    ).bind(&q).bind(status.as_deref()).fetch_one(&state.db).await
     .map_err(|e| ApiError::internal(e.to_string()))?;

    let rows: Vec<BusinessRow> = sqlx::query_as(
        "SELECT o.id,o.name,o.slug,o.status,o.status_reason,o.status_changed_at,
                (SELECT u.email::text FROM organization_memberships om JOIN users u ON u.id=om.user_id
                 WHERE om.organization_id=o.id AND om.role='owner' AND om.status='active'
                 ORDER BY om.joined_at LIMIT 1) AS owner_email,
                (SELECT count(*)::bigint FROM organization_memberships om WHERE om.organization_id=o.id AND om.status='active') AS member_count,
                (SELECT count(*)::bigint FROM organization_domains d WHERE d.organization_id=o.id) AS domain_count,
                (SELECT count(*)::bigint FROM mailboxes m WHERE m.organization_id=o.id AND m.deleted_at IS NULL) AS mailbox_count,
                (SELECT count(*)::bigint FROM mailboxes m WHERE m.organization_id=o.id AND m.deleted_at IS NULL AND m.status='active') AS active_mailbox_count,
                s.plan_code,p.name AS plan_name,s.status AS subscription_status,s.purchased_mailbox_count,
                s.current_period_end,s.assignment_source,o.created_at
         FROM organizations o
         LEFT JOIN organization_subscriptions s ON s.organization_id=o.id
         LEFT JOIN plans p ON p.code=s.plan_code
         WHERE o.is_system=FALSE
           AND ($1::text='' OR o.name ILIKE '%'||$1||'%' OR o.slug ILIKE '%'||$1||'%'
             OR EXISTS(SELECT 1 FROM organization_memberships om JOIN users u ON u.id=om.user_id
                       WHERE om.organization_id=o.id AND u.email::text ILIKE '%'||$1||'%'))
           AND ($2::text IS NULL OR o.status=$2)
         ORDER BY o.created_at DESC,o.id DESC LIMIT $3 OFFSET $4"
    ).bind(&q).bind(status.as_deref()).bind(limit).bind(offset).fetch_all(&state.db).await
     .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({"businesses": rows.iter().map(|r| json!({
        "id":r.id,"name":r.name,"slug":r.slug,"status":r.status,"status_reason":r.status_reason,
        "status_changed_at":r.status_changed_at,"owner_email":r.owner_email,"member_count":r.member_count,
        "domain_count":r.domain_count,"mailbox_count":r.mailbox_count,"active_mailbox_count":r.active_mailbox_count,
        "plan_code":r.plan_code,"plan_name":r.plan_name,"subscription_status":r.subscription_status,
        "purchased_mailbox_count":r.purchased_mailbox_count,"current_period_end":r.current_period_end,
        "assignment_source":r.assignment_source,"created_at":r.created_at
    })).collect::<Vec<_>>(),"total":total,"limit":limit,"offset":offset})))
}

#[derive(Deserialize)]
pub struct BusinessStatusPatch {
    status: String,
    #[serde(default)] reason: String,
    #[serde(default)] confirm_name: String,
}

pub async fn update_business_status(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(organization_id): Path<Uuid>,
    Json(body): Json<BusinessStatusPatch>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(body.status.as_str(), "active" | "suspended" | "closed") {
        return Err(ApiError::bad_request("Business status must be active, suspended or closed"));
    }
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(String, bool, String)> = sqlx::query_as(
        "SELECT name,is_system,status FROM organizations WHERE id=$1 FOR UPDATE"
    ).bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let (name, is_system, previous) = row.ok_or_else(|| ApiError::not_found("Business not found"))?;
    if is_system { return Err(ApiError::forbidden("The protected system organization cannot be modified here")); }
    if body.status == "closed" {
        if body.confirm_name.trim() != name {
            return Err(ApiError::bad_request("Enter the exact business name to close this business"));
        }
        let subscription_status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM organization_subscriptions WHERE organization_id=$1 FOR UPDATE"
        )
        .bind(organization_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if subscription_status.as_deref().is_some_and(|status| status != "cancelled") {
            return Err(ApiError::conflict(
                "Cancel this business subscription in Payments & plans before closing the business"
            ));
        }
    }
    let reason = body.reason.trim().chars().take(500).collect::<String>();
    sqlx::query(
        "UPDATE organizations SET status=$2,status_reason=$3,status_changed_by=$4,status_changed_at=now(),updated_at=now() WHERE id=$1"
    ).bind(organization_id).bind(&body.status).bind(&reason).bind(admin.0.user_id)
     .execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let mailbox_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND status <> 'deleting'"
    ).bind(organization_id).fetch_all(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    for mailbox_id in mailbox_ids {
        state.provisioning.enqueue_mailbox_access_tx(&mut tx, mailbox_id).await?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    audit::record(&state, Some(admin.0.user_id), "platform.business.status", json!({
        "organization_id":organization_id,"name":name,"from":previous,"to":body.status,"reason":reason
    })).await;
    Ok(Json(json!({"ok":true,"organization_id":organization_id,"status":body.status})))
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    user_id: Uuid,
    email: String,
    display_name: String,
    role: String,
    status: String,
    platform_role: String,
    joined_at: DateTime<Utc>,
}

pub async fn business_members(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let system: Option<bool> = sqlx::query_scalar("SELECT is_system FROM organizations WHERE id=$1")
        .bind(organization_id).fetch_optional(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let system=system.ok_or_else(||ApiError::not_found("Business not found"))?;
    if system { return Err(ApiError::forbidden("The protected system organization is not managed through the customer control plane")); }
    let rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT u.id AS user_id,u.email::text AS email,u.display_name,om.role,om.status,u.platform_role,om.joined_at
         FROM organization_memberships om JOIN users u ON u.id=om.user_id
         WHERE om.organization_id=$1 ORDER BY CASE om.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'billing' THEN 2 ELSE 3 END,lower(u.email::text)"
    ).bind(organization_id).fetch_all(&state.db).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"members":rows.iter().map(|r|json!({"user_id":r.user_id,"email":r.email,"display_name":r.display_name,"role":r.role,"status":r.status,"platform_role":r.platform_role,"joined_at":r.joined_at})).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct MemberPatch { role: String, status: String }

async fn ensure_owner_survives(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    user_id: Uuid,
    new_role: Option<&str>,
    new_status: Option<&str>,
    deleting: bool,
) -> Result<(), ApiError> {
    let current: Option<(String,String)> = sqlx::query_as(
        "SELECT role,status FROM organization_memberships WHERE organization_id=$1 AND user_id=$2 FOR UPDATE"
    ).bind(organization_id).bind(user_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((role,status)) = current else { return Err(ApiError::not_found("Business member not found")); };
    let loses_owner = role == "owner" && status == "active" && (deleting || new_role.is_some_and(|v| v != "owner") || new_status.is_some_and(|v| v != "active"));
    if loses_owner {
        let others: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM organization_memberships WHERE organization_id=$1 AND role='owner' AND status='active' AND user_id<>$2"
        ).bind(organization_id).bind(user_id).fetch_one(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        if others == 0 { return Err(ApiError::conflict("Promote another active owner before changing the sole owner")); }
    }
    Ok(())
}

pub async fn update_business_member(
    State(state): State<AppState>, admin: AdminUser,
    Path((organization_id,user_id)): Path<(Uuid,Uuid)>, Json(body): Json<MemberPatch>,
) -> Result<Json<Value>,ApiError> {
    if !matches!(body.role.as_str(),"owner"|"admin"|"billing"|"member") { return Err(ApiError::bad_request("Business role is invalid")); }
    if !matches!(body.status.as_str(),"active"|"suspended") { return Err(ApiError::bad_request("Membership status is invalid")); }
    let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let is_system:Option<bool>=sqlx::query_scalar("SELECT is_system FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    match is_system { Some(false)=>{}, Some(true)=>return Err(ApiError::forbidden("The protected system organization cannot be modified here")), None=>return Err(ApiError::not_found("Business not found")) }
    ensure_owner_survives(&mut tx,organization_id,user_id,Some(&body.role),Some(&body.status),false).await?;
    let changed=sqlx::query("UPDATE organization_memberships SET role=$3,status=$4,updated_at=now() WHERE organization_id=$1 AND user_id=$2")
        .bind(organization_id).bind(user_id).bind(&body.role).bind(&body.status).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected();
    if changed==0 { return Err(ApiError::not_found("Business member not found")); }
    let mailbox_ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM mailboxes WHERE organization_id=$1 AND user_id=$2 AND deleted_at IS NULL AND status <> 'deleting'")
        .bind(organization_id).bind(user_id).fetch_all(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    for mailbox_id in mailbox_ids { state.provisioning.enqueue_mailbox_access_tx(&mut tx,mailbox_id).await?; }
    tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(admin.0.user_id),"platform.business.member",json!({"organization_id":organization_id,"user_id":user_id,"role":body.role,"status":body.status})).await;
    Ok(Json(json!({"ok":true})))
}

pub async fn remove_business_member(
    State(state): State<AppState>, admin: AdminUser,
    Path((organization_id,user_id)): Path<(Uuid,Uuid)>,
) -> Result<Json<Value>,ApiError> {
    let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let is_system:Option<bool>=sqlx::query_scalar("SELECT is_system FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    match is_system { Some(false)=>{}, Some(true)=>return Err(ApiError::forbidden("The protected system organization cannot be modified here")), None=>return Err(ApiError::not_found("Business not found")) }
    ensure_owner_survives(&mut tx,organization_id,user_id,None,None,true).await?;
    let assigned:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM mailboxes WHERE organization_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(organization_id).bind(user_id).fetch_one(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if assigned>0 { return Err(ApiError::conflict("Reassign or delete this member's hosted mailboxes before removing business access")); }
    sqlx::query("DELETE FROM organization_memberships WHERE organization_id=$1 AND user_id=$2")
        .bind(organization_id).bind(user_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    sqlx::query("UPDATE users SET active_organization_id=NULL,active_mailbox_id=NULL,updated_at=now() WHERE id=$1 AND active_organization_id=$2")
        .bind(user_id).bind(organization_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(admin.0.user_id),"platform.business.member_remove",json!({"organization_id":organization_id,"user_id":user_id})).await;
    Ok(Json(json!({"ok":true})))
}

#[derive(sqlx::FromRow)]
struct DomainAdminRow {
    id: Uuid,
    organization_id: Uuid,
    organization_name: String,
    domain: String,
    status: String,
    is_primary: bool,
    verified_at: Option<DateTime<Utc>>,
    provider_domain_id: Option<String>,
    dns_ready: bool,
    dns_mx_ready: bool,
    dns_spf_ready: bool,
    dns_dkim_ready: bool,
    dns_dmarc_ready: bool,
    last_error: String,
    last_dns_readiness_check: Option<DateTime<Utc>>,
    mailbox_count: i64,
    created_at: DateTime<Utc>,
}

pub async fn domains(
    State(state): State<AppState>, _admin: AdminUser, Query(query): Query<ListQuery>,
) -> Result<Json<Value>,ApiError> {
    let q=query.q.unwrap_or_default().trim().chars().take(200).collect::<String>();
    let status=query.status.filter(|v|!v.trim().is_empty());
    let (limit,offset)=page(query.limit,query.offset);
    let total:i64=sqlx::query_scalar(
        "SELECT count(*)::bigint FROM organization_domains d JOIN organizations o ON o.id=d.organization_id
         WHERE d.is_system=FALSE AND ($1::text='' OR d.domain::text ILIKE '%'||$1||'%' OR o.name ILIKE '%'||$1||'%') AND ($2::text IS NULL OR d.status=$2)"
    ).bind(&q).bind(status.as_deref()).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let rows:Vec<DomainAdminRow>=sqlx::query_as(
        "SELECT d.id,d.organization_id,o.name AS organization_name,d.domain::text AS domain,d.status,d.is_primary,d.verified_at,d.provider_domain_id,
                d.dns_ready,d.dns_mx_ready,d.dns_spf_ready,d.dns_dkim_ready,d.dns_dmarc_ready,d.last_error,d.last_dns_readiness_check,
                (SELECT count(*)::bigint FROM mailboxes m WHERE m.domain_id=d.id AND m.deleted_at IS NULL) AS mailbox_count,d.created_at
         FROM organization_domains d JOIN organizations o ON o.id=d.organization_id
         WHERE d.is_system=FALSE AND ($1::text='' OR d.domain::text ILIKE '%'||$1||'%' OR o.name ILIKE '%'||$1||'%') AND ($2::text IS NULL OR d.status=$2)
         ORDER BY d.created_at DESC,d.id DESC LIMIT $3 OFFSET $4"
    ).bind(&q).bind(status.as_deref()).bind(limit).bind(offset).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"domains":rows.iter().map(|r|json!({
        "id":r.id,"organization_id":r.organization_id,"organization_name":r.organization_name,"domain":r.domain,"status":r.status,"is_primary":r.is_primary,
        "verified_at":r.verified_at,"provider_domain_id":r.provider_domain_id,"dns_ready":r.dns_ready,"dns":{"mx":r.dns_mx_ready,"spf":r.dns_spf_ready,"dkim":r.dns_dkim_ready,"dmarc":r.dns_dmarc_ready},
        "last_error":r.last_error,"last_dns_readiness_check":r.last_dns_readiness_check,"mailbox_count":r.mailbox_count,"created_at":r.created_at
    })).collect::<Vec<_>>(),"total":total,"limit":limit,"offset":offset})))
}

#[derive(Deserialize)]
pub struct DomainAction { action: String, #[serde(default)] confirm_domain: String }

pub async fn domain_action(
    State(state): State<AppState>,admin:AdminUser,Path(domain_id):Path<Uuid>,Json(body):Json<DomainAction>
)->Result<Json<Value>,ApiError>{
    let row:Option<(Uuid,String,String,bool,Option<String>,bool,Option<DateTime<Utc>>,String)>=sqlx::query_as(
        "SELECT organization_id,domain::text,status,is_system,provider_domain_id,dns_ready,verified_at,provider_marker FROM organization_domains WHERE id=$1"
    ).bind(domain_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let (organization_id,domain,status,is_system,provider_domain_id,dns_ready,verified_at,provider_marker)=row.ok_or_else(||ApiError::not_found("Domain not found"))?;
    if is_system{return Err(ApiError::forbidden("The protected system domain cannot be managed here"));}
    match body.action.as_str(){
        "suspend"=>{
            let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
            sqlx::query("UPDATE organization_domains SET status='suspended',updated_at=now() WHERE id=$1").bind(domain_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM mailboxes WHERE domain_id=$1 AND deleted_at IS NULL AND status <> 'deleting'").bind(domain_id).fetch_all(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            for id in ids{state.provisioning.enqueue_mailbox_access_tx(&mut tx,id).await?;}
            tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
        }
        "resume"=>{
            let next=if provider_domain_id.as_deref().is_some_and(|v|!v.trim().is_empty())&&dns_ready{"active"}else if provider_domain_id.as_deref().is_some_and(|v|!v.trim().is_empty()){"dns_pending"}else if verified_at.is_some(){"verified"}else{"pending_verification"};
            let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
            sqlx::query("UPDATE organization_domains SET status=$2,next_dns_check_at=CASE WHEN $2='dns_pending' THEN now() ELSE next_dns_check_at END,updated_at=now() WHERE id=$1").bind(domain_id).bind(next).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM mailboxes WHERE domain_id=$1 AND deleted_at IS NULL AND status <> 'deleting'").bind(domain_id).fetch_all(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
            for id in ids{state.provisioning.enqueue_mailbox_access_tx(&mut tx,id).await?;}
            tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
        }
        "check_dns"=>{domain_onboarding::refresh_one(&state,domain_id).await.map_err(|_|ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"dns_unavailable","DNS readiness check is temporarily unavailable"))?;}
        "delete"=>{
            if body.confirm_domain.trim()!=domain { return Err(ApiError::bad_request("Enter the exact domain name to release it")); }
            let mailbox_count:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM mailboxes WHERE domain_id=$1 AND deleted_at IS NULL")
                .bind(domain_id).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            let address_count:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM business_addresses WHERE domain_id=$1 AND deleted_at IS NULL")
                .bind(domain_id).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            if mailbox_count>0||address_count>0 { return Err(ApiError::conflict("Remove all mailboxes, aliases and groups on this domain before releasing it")); }
            if let Some(provider_id)=provider_domain_id.as_deref().filter(|value|!value.trim().is_empty()) {
                if provider_marker.trim().is_empty(){ return Err(ApiError::conflict("Provider ownership metadata is missing; reconcile this domain before removal")); }
                sqlx::query("UPDATE organization_domains SET status='removing',last_error='',updated_at=now() WHERE id=$1")
                    .bind(domain_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
                if let Err(error)=state.stalwart.delete_customer_domain(provider_id,&domain,&provider_marker).await {
                    tracing::error!(organization_id=%organization_id,domain_id=%domain_id,domain=%domain,provider_domain_id=%provider_id,error=%error,"platform domain removal failed");
                    sqlx::query("UPDATE organization_domains SET status='failed',last_error='Mail-provider domain removal failed. Retry before releasing this claim.',updated_at=now() WHERE id=$1")
                        .bind(domain_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
                    return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"mail_provider",error.public_message()));
                }
            } else if matches!(status.as_str(),"provisioning"|"removing") {
                return Err(ApiError::conflict("Domain provisioning/removal is still in progress"));
            }
            sqlx::query("DELETE FROM organization_domains WHERE id=$1 AND organization_id=$2")
                .bind(domain_id).bind(organization_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        }
        "provision"=>{
            if verified_at.is_none(){return Err(ApiError::bad_request("Verify domain ownership before provisioning"));}
            if !matches!(status.as_str(),"verified"|"failed"|"provisioning"){return Err(ApiError::conflict("Domain is not ready for provisioning"));}
            let marker=if provider_marker.trim().is_empty(){state.stalwart.ownership_marker("domain",&organization_id.to_string(),&domain_id.to_string())}else{provider_marker};
            sqlx::query("UPDATE organization_domains SET status='provisioning',provider_marker=$2,last_error='',updated_at=now() WHERE id=$1").bind(domain_id).bind(&marker).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            let snapshot=match state.stalwart.ensure_customer_domain(provider_domain_id.as_deref(),&domain,&marker).await {
                Ok(snapshot)=>snapshot,
                Err(error)=>{
                    let detail=error.to_string();
                    tracing::error!(organization_id=%organization_id,domain_id=%domain_id,domain=%domain,%detail,"platform domain provisioning failed");
                    let ownership_conflict=detail.contains("already exists")||detail.contains("does not belong");
                    let failure_message=if ownership_conflict {
                        "This domain already exists in the shared mail provider under another ownership marker. A platform operator must inspect it before this business can use it."
                    } else {
                        "Mail-provider provisioning failed. The operation is safe to retry."
                    };
                    sqlx::query("UPDATE organization_domains SET status='failed',last_error=$2,updated_at=now() WHERE id=$1")
                        .bind(domain_id).bind(failure_message)
                        .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
                    if ownership_conflict{
                        return Err(ApiError::conflict(failure_message));
                    }
                    return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"mail_provider",error.public_message()));
                }
            };
            if snapshot.name!=domain||snapshot.description!=marker||!snapshot.enabled{
                sqlx::query("UPDATE organization_domains SET status='failed',last_error='Mail-provider domain ownership could not be established safely',updated_at=now() WHERE id=$1")
                    .bind(domain_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
                return Err(ApiError::conflict("Mail-provider domain ownership could not be established safely"));
            }
            let expected=domain_onboarding::parse_required_records(&snapshot.dns_zone_file,&domain);
            if expected.is_empty(){
                sqlx::query("UPDATE organization_domains SET provider_domain_id=$2,provider_synced_at=now(),dns_zone_file=$3,dns_expected=$4,status='failed',last_error='Mail provider did not generate the required DNS zone records',updated_at=now() WHERE id=$1")
                    .bind(domain_id).bind(&snapshot.id).bind(&snapshot.dns_zone_file).bind(json!(expected)).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
                return Err(ApiError::new(axum::http::StatusCode::BAD_GATEWAY,"mail_provider_dns","Mail provider did not generate DNS setup records"));
            }
            sqlx::query("UPDATE organization_domains SET provider_domain_id=$2,provider_synced_at=now(),dns_zone_file=$3,dns_expected=$4,status='dns_pending',next_dns_check_at=now(),last_error='',updated_at=now() WHERE id=$1")
                .bind(domain_id).bind(&snapshot.id).bind(&snapshot.dns_zone_file).bind(json!(expected)).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            let _=domain_onboarding::refresh_one(&state,domain_id).await;
        }
        _=>return Err(ApiError::bad_request("Domain action is invalid")),
    }
    audit::record(&state,Some(admin.0.user_id),"platform.domain.action",json!({"domain_id":domain_id,"organization_id":organization_id,"domain":domain,"action":body.action})).await;
    Ok(Json(json!({"ok":true,"domain_id":domain_id})))
}

#[derive(sqlx::FromRow)]
struct MailboxAdminRow {
    id:Uuid,organization_id:Uuid,organization_name:String,domain:String,address:String,display_name:String,status:String,
    sync_status:String,sync_error:String,quota_bytes:i64,quota_used:Option<i64>,provider_account_id:Option<String>,user_id:Option<Uuid>,user_email:Option<String>,created_at:DateTime<Utc>
}

pub async fn mailboxes(State(state):State<AppState>,_admin:AdminUser,Query(query):Query<ListQuery>)->Result<Json<Value>,ApiError>{
    let q=query.q.unwrap_or_default().trim().chars().take(200).collect::<String>();let status=query.status.filter(|v|!v.trim().is_empty());let(limit,offset)=page(query.limit,query.offset);
    let total:i64=sqlx::query_scalar("SELECT count(*)::bigint FROM mailboxes m JOIN organizations o ON o.id=m.organization_id JOIN organization_domains d ON d.id=m.domain_id LEFT JOIN users u ON u.id=m.user_id WHERE o.is_system=FALSE AND m.deleted_at IS NULL AND ($1::text='' OR m.address::text ILIKE '%'||$1||'%' OR o.name ILIKE '%'||$1||'%' OR d.domain::text ILIKE '%'||$1||'%' OR COALESCE(u.email::text,'') ILIKE '%'||$1||'%') AND ($2::text IS NULL OR m.status=$2)")
        .bind(&q).bind(status.as_deref()).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let rows:Vec<MailboxAdminRow>=sqlx::query_as("SELECT m.id,m.organization_id,o.name AS organization_name,d.domain::text AS domain,m.address::text AS address,m.display_name,m.status,m.sync_status,m.sync_error,m.quota_bytes,r.quota_used,m.provider_account_id,m.user_id,u.email::text AS user_email,m.created_at FROM mailboxes m JOIN organizations o ON o.id=m.organization_id JOIN organization_domains d ON d.id=m.domain_id LEFT JOIN users u ON u.id=m.user_id LEFT JOIN realtime_mailbox_state r ON r.mailbox_id=m.id WHERE o.is_system=FALSE AND m.deleted_at IS NULL AND ($1::text='' OR m.address::text ILIKE '%'||$1||'%' OR o.name ILIKE '%'||$1||'%' OR d.domain::text ILIKE '%'||$1||'%' OR COALESCE(u.email::text,'') ILIKE '%'||$1||'%') AND ($2::text IS NULL OR m.status=$2) ORDER BY m.created_at DESC,m.id DESC LIMIT $3 OFFSET $4")
        .bind(&q).bind(status.as_deref()).bind(limit).bind(offset).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"mailboxes":rows.iter().map(|r|json!({"id":r.id,"organization_id":r.organization_id,"organization_name":r.organization_name,"domain":r.domain,"address":r.address,"display_name":r.display_name,"status":r.status,"sync_status":r.sync_status,"sync_error":r.sync_error,"quota_bytes":r.quota_bytes,"quota_used":r.quota_used,"provider_account_id":r.provider_account_id,"user_id":r.user_id,"user_email":r.user_email,"created_at":r.created_at})).collect::<Vec<_>>(),"total":total,"limit":limit,"offset":offset})))
}

#[derive(Deserialize)]pub struct MailboxAction{action:String,#[serde(default)]confirm_address:String,quota_bytes:Option<i64>,#[serde(default)]reset_to_default:bool}
pub async fn mailbox_action(State(state):State<AppState>,admin:AdminUser,Path(mailbox_id):Path<Uuid>,Json(body):Json<MailboxAction>)->Result<Json<Value>,ApiError>{
    let row:Option<(Uuid,String,String,bool)>=sqlx::query_as("SELECT m.organization_id,m.address::text,m.status,o.is_system FROM mailboxes m JOIN organizations o ON o.id=m.organization_id WHERE m.id=$1 AND m.deleted_at IS NULL").bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let (organization_id,address,_status,is_system)=row.ok_or_else(||ApiError::not_found("Mailbox not found"))?;
    if is_system{return Err(ApiError::forbidden("Protected system mailboxes are not managed through the customer control plane"));}
    match body.action.as_str(){
      "suspend"|"activate"=>{let next=if body.action=="suspend"{"suspended"}else{"active"};let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;sqlx::query("UPDATE mailboxes SET status=$2,suspended_at=CASE WHEN $2='suspended' THEN now() ELSE NULL END,updated_at=now() WHERE id=$1").bind(mailbox_id).bind(next).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;state.provisioning.enqueue_mailbox_access_tx(&mut tx,mailbox_id).await?;tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;}
      "set_quota"=>{
        if body.reset_to_default==body.quota_bytes.is_some(){return Err(ApiError::bad_request("Provide quota_bytes or reset_to_default, but not both"));}
        let ent=crate::services::entitlements::for_organization(&state,organization_id).await?;
        let default_quota=ent.quota_bytes.min(i64::MAX as u64) as i64;
        let target=if body.reset_to_default{default_quota}else{body.quota_bytes.unwrap_or(0)};
        if target<1_048_576{return Err(ApiError::bad_request("Mailbox storage allocation must be at least 1 MiB"));}
        if target as u64>ent.storage_pool_bytes{return Err(ApiError::bad_request("A mailbox allocation cannot exceed the business storage pool"));}
        let quota_row:Option<(Option<String>,i64)>=sqlx::query_as("SELECT provider_account_id,quota_bytes FROM mailboxes WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL AND status<>'deleting'")
            .bind(mailbox_id).bind(organization_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        let (provider_account_id,current_quota)=quota_row.ok_or_else(||ApiError::not_found("Mailbox not found"))?;
        let cached_used:Option<i64>=sqlx::query_scalar("SELECT quota_used FROM realtime_mailbox_state WHERE mailbox_id=$1").bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.flatten();
        let live_used=if let Some(account)=provider_account_id.as_deref().filter(|value|!value.trim().is_empty()){match state.stalwart.account_quota(account).await{Ok(Some((used,_)))=>Some(used.min(i64::MAX as u64) as i64),Ok(None)=>None,Err(_)=>None}}else{None};
        let known_used=live_used.or(cached_used).map(|value|value.max(0));
        if target<current_quota&&known_used.is_none(){return Err(ApiError::conflict("Current mailbox usage could not be verified. Retry after provider usage is available before lowering this allocation"));}
        if target<known_used.unwrap_or(0){return Err(ApiError::conflict("Mailbox allocation cannot be lowered below its current storage usage"));}
        let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
        let pool:i64=sqlx::query_scalar("SELECT COALESCE(s.storage_pool_override_bytes,p.storage_pool_bytes::bigint+p.mailbox_bytes::bigint*GREATEST(s.purchased_mailbox_count-p.mailbox_limit,0)::bigint)::bigint FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code WHERE s.organization_id=$1 FOR UPDATE OF s")
            .bind(organization_id).fetch_optional(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?.ok_or_else(||ApiError::forbidden("Business subscription is not available"))?;
        let allocated_other:i64=sqlx::query_scalar("SELECT COALESCE(SUM(quota_bytes),0)::bigint FROM mailboxes WHERE organization_id=$1 AND deleted_at IS NULL AND id<>$2")
            .bind(organization_id).bind(mailbox_id).fetch_one(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
        if allocated_other.saturating_add(target)>pool{return Err(ApiError::conflict(format!("Only {} bytes remain available in the business storage pool",pool.saturating_sub(allocated_other))));}
        sqlx::query("UPDATE mailboxes SET quota_bytes=$3,quota_override_bytes=$4,quota_updated_by=$5,quota_updated_at=now(),updated_at=now() WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL")
            .bind(mailbox_id).bind(organization_id).bind(target).bind(if body.reset_to_default{None::<i64>}else{Some(target)}).bind(admin.0.user_id)
            .execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
        state.provisioning.enqueue_mailbox_quota_tx(&mut tx,mailbox_id,target).await?;
        tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
      }
      "delete"=>{if body.confirm_address.trim()!=address{return Err(ApiError::bad_request("Enter the exact mailbox address to delete it"));}let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_address_members WHERE mailbox_id=$1)").bind(mailbox_id).fetch_one(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;if used{return Err(ApiError::conflict("Remove this mailbox from aliases/groups before deleting it"));}sqlx::query("UPDATE mailboxes SET status='deleting',updated_at=now() WHERE id=$1").bind(mailbox_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;sqlx::query("UPDATE users SET active_mailbox_id=CASE WHEN active_mailbox_id=$1 THEN NULL ELSE active_mailbox_id END,primary_mailbox_id=CASE WHEN primary_mailbox_id=$1 THEN NULL ELSE primary_mailbox_id END,updated_at=now() WHERE active_mailbox_id=$1 OR primary_mailbox_id=$1").bind(mailbox_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;state.provisioning.enqueue_mailbox_delete_tx(&mut tx,mailbox_id).await?;tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;}
      _=>return Err(ApiError::bad_request("Mailbox action is invalid")),
    }
    audit::record(&state,Some(admin.0.user_id),"platform.mailbox.action",json!({"mailbox_id":mailbox_id,"organization_id":organization_id,"address":address,"action":body.action,"quota_bytes":body.quota_bytes,"reset_to_default":body.reset_to_default})).await;
    Ok(Json(json!({"ok":true,"mailbox_id":mailbox_id})))
}

#[derive(Deserialize,Default)]pub struct RecoveryQuery{kind:Option<String>,limit:Option<i64>}
pub async fn recovery(State(state):State<AppState>,_admin:AdminUser,Query(query):Query<RecoveryQuery>)->Result<Json<Value>,ApiError>{
    let limit=query.limit.unwrap_or(100).clamp(1,200);let kind=query.kind.unwrap_or_default();
    let provisioning:Vec<Value>=if kind.is_empty()||kind=="provisioning"{sqlx::query_as::<_,(Uuid,String,String,String,i32,i32,String,DateTime<Utc>,Option<Uuid>)>("SELECT id,operation,target_email::text,status,attempts,max_attempts,last_error,updated_at,mailbox_id FROM provisioning_jobs WHERE status IN ('retry','processing','dead') ORDER BY updated_at DESC LIMIT $1").bind(limit).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.into_iter().map(|r|json!({"id":r.0,"kind":"provisioning","operation":r.1,"target":r.2,"status":r.3,"attempts":r.4,"max_attempts":r.5,"last_error":r.6,"updated_at":r.7,"mailbox_id":r.8})).collect()}else{vec![]};
    let imports:Vec<Value>=if kind.is_empty()||kind=="imports"{sqlx::query_as::<_,(Uuid,String,String,i32,i32,String,DateTime<Utc>)>("SELECT mi.id,m.address::text,mi.status,mi.attempts,mi.max_attempts,mi.last_error,mi.updated_at FROM mailbox_imports mi JOIN mailboxes m ON m.id=mi.mailbox_id WHERE mi.status IN ('queued','running','failed') ORDER BY mi.updated_at DESC LIMIT $1").bind(limit).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.into_iter().map(|r|json!({"id":r.0,"kind":"import","target":r.1,"status":r.2,"attempts":r.3,"max_attempts":r.4,"last_error":r.5,"updated_at":r.6})).collect()}else{vec![]};
    let scheduled:Vec<Value>=if kind.is_empty()||kind=="scheduled"{sqlx::query_as::<_,(Uuid,String,String,i32,String,DateTime<Utc>)>("SELECT s.id,m.address::text,s.status,s.attempt_count,s.error,s.updated_at FROM scheduled_sends s JOIN mailboxes m ON m.id=s.mailbox_id WHERE s.status IN ('processing','retry','dead') ORDER BY s.updated_at DESC LIMIT $1").bind(limit).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.into_iter().map(|r|json!({"id":r.0,"kind":"scheduled","target":r.1,"status":r.2,"attempts":r.3,"last_error":r.4,"updated_at":r.5})).collect()}else{vec![]};
    let billing:Vec<Value>=if kind.is_empty()||kind=="billing"{sqlx::query_as::<_,(Uuid,String,String,i32,String,DateTime<Utc>)>("SELECT b.id,b.recipient::text,b.status,b.attempts,b.last_error,b.updated_at FROM billing_email_outbox b WHERE b.status IN ('pending','sending','retry','failed') ORDER BY b.updated_at DESC LIMIT $1").bind(limit).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.into_iter().map(|r|json!({"id":r.0,"kind":"billing_email","target":r.1,"status":r.2,"attempts":r.3,"last_error":r.4,"updated_at":r.5})).collect()}else{vec![]};
    Ok(Json(json!({"provisioning":provisioning,"imports":imports,"scheduled":scheduled,"billing_email":billing})))
}

#[derive(Deserialize)]pub struct RetryAction{kind:String}
pub async fn retry_recovery(State(state):State<AppState>,admin:AdminUser,Path(id):Path<Uuid>,Json(body):Json<RetryAction>)->Result<Json<Value>,ApiError>{
    let affected=match body.kind.as_str(){
      "provisioning"=>sqlx::query("UPDATE provisioning_jobs SET status='retry',attempts=0,next_attempt_at=now(),locked_at=NULL,locked_by=NULL,last_error='',completed_at=NULL,updated_at=now() WHERE id=$1 AND status IN ('retry','dead')").bind(id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected(),
      "import"=>sqlx::query("UPDATE mailbox_imports SET status='queued',attempts=0,next_attempt_at=now(),locked_by=NULL,locked_until=NULL,last_error='',completed_at=NULL,updated_at=now() WHERE id=$1 AND status='failed'").bind(id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected(),
      "scheduled"=>sqlx::query("UPDATE scheduled_sends SET status='retry',attempt_count=0,next_attempt_at=now(),claimed_by=NULL,lease_until=NULL,error='',completed_at=NULL,updated_at=now() WHERE id=$1 AND status='dead'").bind(id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected(),
      "billing_email"=>sqlx::query("UPDATE billing_email_outbox SET status='retry',attempts=0,next_attempt_at=now(),last_error='',sent_at=NULL,updated_at=now() WHERE id=$1 AND status='failed'").bind(id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected(),
      _=>return Err(ApiError::bad_request("Recovery job kind is invalid")),
    };
    if affected==0{return Err(ApiError::conflict("This job is not currently retryable"));}
    audit::record(&state,Some(admin.0.user_id),"platform.recovery.retry",json!({"kind":body.kind,"id":id})).await;
    Ok(Json(json!({"ok":true})))
}

pub async fn controls(State(state):State<AppState>,_admin:AdminUser)->Result<Json<Value>,ApiError>{
    let c=platform_control::load(&state.db).await?;
    let updated_by_email:Option<String>=if let Some(user_id)=c.updated_by {
        sqlx::query_scalar("SELECT email::text FROM users WHERE id=$1").bind(user_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?
    } else { None };
    Ok(Json(json!({"public_signup_enabled":c.public_signup_enabled,"business_creation_enabled":c.business_creation_enabled,"plan_ordering_enabled":c.plan_ordering_enabled,"domain_onboarding_enabled":c.domain_onboarding_enabled,"mailbox_provisioning_enabled":c.mailbox_provisioning_enabled,"outbound_sending_enabled":c.outbound_sending_enabled,"maintenance_message":c.maintenance_message,"updated_at":c.updated_at,"updated_by":c.updated_by,"updated_by_email":updated_by_email})))
}

#[derive(Deserialize)]pub struct ControlPatch{public_signup_enabled:bool,business_creation_enabled:bool,plan_ordering_enabled:bool,domain_onboarding_enabled:bool,mailbox_provisioning_enabled:bool,outbound_sending_enabled:bool,#[serde(default)]maintenance_message:String}
pub async fn update_controls(State(state):State<AppState>,admin:AdminUser,Json(body):Json<ControlPatch>)->Result<Json<Value>,ApiError>{
    let message=body.maintenance_message.trim().chars().take(500).collect::<String>();
    sqlx::query("UPDATE platform_controls SET public_signup_enabled=$1,business_creation_enabled=$2,plan_ordering_enabled=$3,domain_onboarding_enabled=$4,mailbox_provisioning_enabled=$5,outbound_sending_enabled=$6,maintenance_message=$7,updated_by=$8,updated_at=now() WHERE singleton=TRUE")
      .bind(body.public_signup_enabled).bind(body.business_creation_enabled).bind(body.plan_ordering_enabled).bind(body.domain_onboarding_enabled).bind(body.mailbox_provisioning_enabled).bind(body.outbound_sending_enabled).bind(&message).bind(admin.0.user_id)
      .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(admin.0.user_id),"platform.controls.update",json!({"public_signup_enabled":body.public_signup_enabled,"business_creation_enabled":body.business_creation_enabled,"plan_ordering_enabled":body.plan_ordering_enabled,"domain_onboarding_enabled":body.domain_onboarding_enabled,"mailbox_provisioning_enabled":body.mailbox_provisioning_enabled,"outbound_sending_enabled":body.outbound_sending_enabled,"maintenance_message":message})).await;
    controls(State(state),admin).await
}
