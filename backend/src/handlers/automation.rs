use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use std::collections::HashSet;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{automation, email, entitlements, tenancy};
use crate::state::AppState;

fn hash_token(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

async fn require_feature(state: &AppState, user_id: Uuid, feature: &str) -> Result<(), ApiError> {
    entitlements::require_feature(state, user_id, feature).await
}

async fn active_mailbox(state: &AppState, auth: &AuthUser) -> Result<tenancy::ActiveMailbox, ApiError> {
    tenancy::active_mailbox(&state.db, auth.user_id, auth.organization_id_hint, auth.mailbox_id_hint)
        .await?
        .ok_or_else(|| ApiError::conflict("Create or select a business mailbox before configuring mail automation"))
}

async fn sync_after_change(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<Value, ApiError> {
    automation::mark_dirty(state, user_id, mailbox_id)
        .await
        .map_err(ApiError::internal)?;
    if let Err(error) = automation::sync_user(state, user_id, mailbox_id).await {
        tracing::warn!(user_id=%user_id, mailbox_id=%mailbox_id, %error, "mail automation change queued for reconciliation");
    }
    automation::sync_status(state, mailbox_id)
        .await
        .map_err(ApiError::internal)
}

fn rule_to_json(
    id: Uuid,
    name: String,
    enabled: bool,
    position: i32,
    conditions: Value,
    actions: Value,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "enabled": enabled,
        "position": position,
        "conditions": conditions,
        "actions": actions,
    })
}

pub async fn list_rules(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_feature(&state, auth.user_id, "mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let rows: Vec<(Uuid, String, bool, i32, Value, Value)> = sqlx::query_as(
        "SELECT id, name, enabled, position, conditions, actions
         FROM mail_rules WHERE mailbox_id=$1 ORDER BY position, created_at, id",
    )
    .bind(mailbox.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let rules = rows
        .into_iter()
        .map(|(id, name, enabled, position, conditions, actions)| {
            rule_to_json(id, name, enabled, position, conditions, actions)
        })
        .collect::<Vec<_>>();
    let sync = automation::sync_status(&state, mailbox.id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "rules": rules, "sync": sync })))
}

#[derive(Deserialize)]
pub struct RuleIn {
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    conditions: Value,
    actions: Value,
}
fn default_true() -> bool { true }

async fn address_is_internal(state: &AppState, address: &str) -> Result<bool, ApiError> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM mailboxes
           WHERE deleted_at IS NULL AND status <> 'deleting'
             AND lower(address::text)=lower($1)
         )",
    )
    .bind(address)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(found)
}

async fn own_mailbox_address(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<String, ApiError> {
    tenancy::active_mailbox(&state.db, user_id, None, Some(mailbox_id))
        .await?
        .map(|mailbox| mailbox.address.to_ascii_lowercase())
        .ok_or_else(|| ApiError::conflict("Create or assign a business mailbox before configuring mail automation"))
}

async fn verified_forward_target(state: &AppState, mailbox_id: Uuid) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar(
        "SELECT target_email::text FROM mail_forwarding
         WHERE mailbox_id=$1 AND verified_at IS NOT NULL AND target_email <> ''",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

async fn validate_rule_forward_targets(
    state: &AppState,
    user_id: Uuid,
    mailbox_id: Uuid,
    actions: &Value,
) -> Result<(), ApiError> {
    let own_email = own_mailbox_address(state, user_id, mailbox_id).await?;
    let verified = verified_forward_target(state, mailbox_id).await?;
    for action in actions.as_array().into_iter().flatten() {
        if action.get("kind").and_then(Value::as_str) != Some("forward") { continue; }
        let target = action.get("value").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        if target.eq_ignore_ascii_case(&own_email) {
            return Err(ApiError::bad_request("A rule cannot forward mail back to the same mailbox"));
        }
        if would_forward_loop(state, user_id, mailbox_id, &target).await? {
            return Err(ApiError::bad_request("This rule would create a forwarding loop"));
        }
        if address_is_internal(state, &target).await? { continue; }
        if verified.as_deref().is_some_and(|value| value.eq_ignore_ascii_case(&target)) { continue; }
        return Err(ApiError::bad_request(
            "External rule forwarding is allowed only to the verified forwarding address",
        ));
    }
    Ok(())
}

pub async fn create_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RuleIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_feature(&state, auth.user_id, "mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    automation::validate_rule(&body.name, &body.conditions, &body.actions)
        .map_err(ApiError::bad_request)?;
    validate_rule_forward_targets(&state, auth.user_id, mailbox.id, &body.actions).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('mail-rules:' || $1, 0))")
        .bind(mailbox.id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM mail_rules WHERE mailbox_id=$1")
        .bind(mailbox.id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if count >= 100 { return Err(ApiError::bad_request("A maximum of 100 rules is supported")); }
    let position: i32 = sqlx::query_scalar("SELECT COALESCE(max(position), -1) + 1 FROM mail_rules WHERE mailbox_id=$1")
        .bind(mailbox.id).fetch_one(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let row: (Uuid, String, bool, i32, Value, Value) = sqlx::query_as(
        "INSERT INTO mail_rules (user_id,mailbox_id,name,enabled,position,conditions,actions)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING id,name,enabled,position,conditions,actions",
    ).bind(auth.user_id).bind(mailbox.id).bind(body.name.trim()).bind(body.enabled).bind(position)
      .bind(&body.conditions).bind(&body.actions).fetch_one(&mut *tx).await
      .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let sync = sync_after_change(&state, auth.user_id, mailbox.id).await?;
    audit::record(&state, Some(auth.user_id), "mail.rule.create", json!({"id":row.0})).await;
    Ok((StatusCode::CREATED, Json(json!({
        "rule": rule_to_json(row.0,row.1,row.2,row.3,row.4,row.5),
        "sync": sync
    }))))
}

pub async fn update_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RuleIn>,
) -> Result<Json<Value>, ApiError> {
    require_feature(&state, auth.user_id, "mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    automation::validate_rule(&body.name, &body.conditions, &body.actions).map_err(ApiError::bad_request)?;
    validate_rule_forward_targets(&state, auth.user_id, mailbox.id, &body.actions).await?;
    let row: Option<(Uuid, String, bool, i32, Value, Value)> = sqlx::query_as(
        "UPDATE mail_rules SET name=$3, enabled=$4, conditions=$5, actions=$6, updated_at=now()
         WHERE mailbox_id=$1 AND id=$2
         RETURNING id,name,enabled,position,conditions,actions",
    ).bind(mailbox.id).bind(id).bind(body.name.trim()).bind(body.enabled)
      .bind(&body.conditions).bind(&body.actions).fetch_optional(&state.db).await
      .map_err(|e| ApiError::internal(e.to_string()))?;
    let row = row.ok_or_else(|| ApiError::not_found("Rule not found"))?;
    let sync = sync_after_change(&state, auth.user_id, mailbox.id).await?;
    audit::record(&state, Some(auth.user_id), "mail.rule.update", json!({"id":id})).await;
    Ok(Json(json!({"rule": rule_to_json(row.0,row.1,row.2,row.3,row.4,row.5), "sync":sync})))
}

pub async fn delete_rule(
    State(state): State<AppState>, auth: AuthUser, Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    require_feature(&state, auth.user_id, "mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('mail-rules:' || $1, 0))")
        .bind(mailbox.id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let deleted = sqlx::query("DELETE FROM mail_rules WHERE mailbox_id=$1 AND id=$2")
        .bind(mailbox.id).bind(id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if deleted.rows_affected()==0 { return Err(ApiError::not_found("Rule not found")); }
    // Compact positions to keep the order stable and human-readable.
    sqlx::query(
        "WITH ranked AS (SELECT id, row_number() OVER (ORDER BY position,created_at,id)-1 AS p FROM mail_rules WHERE mailbox_id=$1)
         UPDATE mail_rules r SET position=ranked.p::int FROM ranked WHERE r.id=ranked.id",
    ).bind(mailbox.id).execute(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let sync = sync_after_change(&state, auth.user_id, mailbox.id).await?;
    audit::record(&state, Some(auth.user_id), "mail.rule.delete", json!({"id":id})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

#[derive(Deserialize)]
pub struct ReorderIn { ids: Vec<Uuid> }

pub async fn reorder_rules(
    State(state): State<AppState>, auth: AuthUser, Json(body): Json<ReorderIn>,
) -> Result<Json<Value>, ApiError> {
    require_feature(&state, auth.user_id, "mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let mut tx = state.db.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('mail-rules:' || $1, 0))")
        .bind(mailbox.id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let existing: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM mail_rules WHERE mailbox_id=$1 ORDER BY position,created_at,id")
        .bind(auth.user_id).fetch_all(&mut *tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
    if body.ids.len()!=existing.len() || existing.iter().any(|id| !body.ids.contains(id)) {
        return Err(ApiError::bad_request("Rule order must contain every rule exactly once"));
    }
    for (position,id) in body.ids.iter().enumerate() {
        sqlx::query("UPDATE mail_rules SET position=$3, updated_at=now() WHERE mailbox_id=$1 AND id=$2")
            .bind(mailbox.id).bind(id).bind(position as i32).execute(&mut *tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    let sync = sync_after_change(&state, auth.user_id, mailbox.id).await?;
    audit::record(&state, Some(auth.user_id), "mail.rule.reorder", json!({"ids":body.ids})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

fn forwarding_json(row: Option<(bool,String,bool,Option<chrono::DateTime<chrono::Utc>>,Option<chrono::DateTime<chrono::Utc>>)>)->Value{
    match row {
        Some((enabled,address,keep_copy,verified_at,expires_at))=>json!({
            "enabled":enabled,"address":address,"keepCopy":keep_copy,
            "verified":verified_at.is_some(),"verifiedAt":verified_at,
            "verificationPending": verified_at.is_none() && !address.is_empty() && expires_at.is_some(),
            "verificationExpiresAt":expires_at
        }),
        None=>json!({"enabled":false,"address":"","keepCopy":true,"verified":false,"verifiedAt":null,"verificationPending":false,"verificationExpiresAt":null})
    }
}

pub async fn get_forwarding(State(state):State<AppState>,auth:AuthUser)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"forwarding").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let row = sqlx::query_as(
        "SELECT enabled,target_email::text,keep_copy,verified_at,verification_expires_at FROM mail_forwarding WHERE mailbox_id=$1"
    ).bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let sync=automation::sync_status(&state,mailbox.id).await.map_err(ApiError::internal)?;
    Ok(Json(json!({"forwarding":forwarding_json(row),"sync":sync})))
}

#[derive(Deserialize)]
pub struct ForwardingIn { enabled:bool, address:String, #[serde(rename="keepCopy")] keep_copy:bool }

async fn outgoing_forward_targets(state: &AppState, address: &str) -> Result<Vec<String>, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "WITH owner AS (
           SELECT id AS mailbox_id FROM mailboxes
            WHERE deleted_at IS NULL AND status <> 'deleting'
              AND user_id IS NOT NULL AND lower(address::text)=lower($1)
            LIMIT 1
         )
         SELECT lower(f.target_email::text)
           FROM owner o JOIN mail_forwarding f ON f.mailbox_id=o.mailbox_id
          WHERE f.enabled AND f.verified_at IS NOT NULL AND f.target_email<>''
         UNION
         SELECT lower(action->>'value')
           FROM owner o
           JOIN mail_rules r ON r.mailbox_id=o.mailbox_id AND r.enabled
           CROSS JOIN LATERAL jsonb_array_elements(r.actions) action
          WHERE action->>'kind'='forward' AND action->>'value'<>''",
    )
    .bind(address)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(rows)
}

/// Follow CS Mail-controlled forwarding edges and fail closed on a cycle or
/// an excessively deep chain. External forwarding outside our authority is
/// still protected by SMTP/Sieve loop safeguards, but cannot be precomputed.
async fn would_forward_loop(state: &AppState, user_id: Uuid, mailbox_id: Uuid, target: &str) -> Result<bool, ApiError> {
    let own = own_mailbox_address(state, user_id, mailbox_id).await?;
    let mut stack = vec![(target.trim().to_lowercase(), 0usize)];
    let mut seen = HashSet::new();
    while let Some((current, depth)) = stack.pop() {
        if current == own { return Ok(true); }
        if depth >= 20 { return Ok(true); }
        if !seen.insert(current.clone()) { continue; }
        for next in outgoing_forward_targets(state, &current).await? {
            stack.push((next, depth + 1));
        }
    }
    Ok(false)
}

async fn issue_forward_code(state:&AppState,mailbox_id:Uuid,target:&str)->Result<Option<String>,ApiError>{
    let raw=email::random_token();
    let code=raw[..12].to_ascii_uppercase();
    let hash=hash_token(&code);
    sqlx::query(
        "UPDATE mail_forwarding SET verification_token_hash=$2,
          verification_expires_at=now()+interval '30 minutes', verification_sent_at=now(), updated_at=now()
         WHERE mailbox_id=$1"
    ).bind(mailbox_id).bind(hash).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    email::send_forwarding_verification(state,target,&code).await?;
    Ok(state.return_token_links.then_some(code))
}

pub async fn put_forwarding(
    State(state):State<AppState>,auth:AuthUser,Json(body):Json<ForwardingIn>
)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"forwarding").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let target=body.address.trim().to_lowercase();
    if target.is_empty() {
        sqlx::query(
            "INSERT INTO mail_forwarding(user_id,mailbox_id,enabled,target_email,keep_copy) VALUES($1,$2,false,'',true)
             ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET enabled=false,target_email='',keep_copy=true,verified_at=NULL,
             verification_token_hash='',verification_expires_at=NULL,verification_sent_at=NULL,updated_at=now()"
        ).bind(auth.user_id).bind(mailbox.id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        let sync=sync_after_change(&state,auth.user_id,mailbox.id).await?;
        return Ok(Json(json!({"forwarding":forwarding_json(Some((false,"".into(),true,None,None))),"sync":sync})))
    }
    if !automation::valid_email(&target) { return Err(ApiError::bad_request("Enter a valid forwarding address")); }
    let own_email = own_mailbox_address(&state, auth.user_id, mailbox.id).await?;
    if target.eq_ignore_ascii_case(&own_email) { return Err(ApiError::bad_request("You cannot forward a mailbox to itself")); }
    if would_forward_loop(&state,auth.user_id,mailbox.id,&target).await? { return Err(ApiError::bad_request("This forwarding path would create a loop")); }

    let existing: Option<(String,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT target_email::text,verified_at FROM mail_forwarding WHERE mailbox_id=$1"
    ).bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let same_verified=existing.as_ref().is_some_and(|(address,verified)| address.eq_ignore_ascii_case(&target)&&verified.is_some());
    sqlx::query(
        "INSERT INTO mail_forwarding(user_id,mailbox_id,enabled,target_email,keep_copy,verified_at)
         VALUES($1,$2,$3,$4,$5,$6)
         ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET enabled=EXCLUDED.enabled,target_email=EXCLUDED.target_email,
           keep_copy=EXCLUDED.keep_copy,verified_at=EXCLUDED.verified_at,
           verification_token_hash=CASE WHEN mail_forwarding.target_email::text<>EXCLUDED.target_email::text THEN '' ELSE mail_forwarding.verification_token_hash END,
           verification_expires_at=CASE WHEN mail_forwarding.target_email::text<>EXCLUDED.target_email::text THEN NULL ELSE mail_forwarding.verification_expires_at END,
           updated_at=now()"
    ).bind(auth.user_id).bind(mailbox.id).bind(body.enabled && same_verified).bind(&target).bind(body.keep_copy)
      .bind(if same_verified { existing.as_ref().and_then(|(_, v)| v.clone()) } else { None })
      .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if !body.enabled && !same_verified {
        sqlx::query(
            "UPDATE mail_forwarding SET verification_token_hash='', verification_expires_at=NULL,
             verification_sent_at=NULL, updated_at=now() WHERE mailbox_id=$1"
        ).bind(mailbox.id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    }

    let mut dev_code=None;
    if body.enabled && !same_verified { dev_code=issue_forward_code(&state,mailbox.id,&target).await?; }
    let sync=sync_after_change(&state,auth.user_id,mailbox.id).await?;
    let row=sqlx::query_as("SELECT enabled,target_email::text,keep_copy,verified_at,verification_expires_at FROM mail_forwarding WHERE mailbox_id=$1")
      .bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"mail.forwarding.update",json!({"enabled":body.enabled,"target":target})).await;
    Ok(Json(json!({"forwarding":forwarding_json(row),"sync":sync,"verificationCode":dev_code})))
}

#[derive(Deserialize)]
pub struct VerifyForwardIn { code:String }
pub async fn verify_forwarding(
    State(state):State<AppState>,auth:AuthUser,Json(body):Json<VerifyForwardIn>
)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"forwarding").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let row: Option<(String,String,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT target_email::text,verification_token_hash,verification_expires_at FROM mail_forwarding WHERE mailbox_id=$1"
    ).bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let (target,hash,expires)=row.ok_or_else(||ApiError::bad_request("No forwarding verification is pending"))?;
    let supplied_hash = hash_token(&body.code.trim().to_ascii_uppercase());
    if hash.is_empty() || expires.as_ref().map_or(true, |at| at <= &chrono::Utc::now()) || supplied_hash!=hash {
        return Err(ApiError::bad_request("Verification code is invalid or expired"));
    }
    if would_forward_loop(&state,auth.user_id,mailbox.id,&target).await? { return Err(ApiError::bad_request("This forwarding path would create a loop")); }
    let verified = sqlx::query(
        "UPDATE mail_forwarding SET enabled=true,verified_at=now(),verification_token_hash='',
         verification_expires_at=NULL,verification_sent_at=NULL,updated_at=now()
         WHERE mailbox_id=$1 AND target_email::text=$2 AND verification_token_hash=$3
           AND verification_expires_at > now()"
    ).bind(mailbox.id).bind(&target).bind(&supplied_hash).execute(&state.db).await
      .map_err(|e|ApiError::internal(e.to_string()))?;
    if verified.rows_affected() != 1 {
        return Err(ApiError::bad_request("Verification code is invalid or expired"));
    }
    let sync=sync_after_change(&state,auth.user_id,mailbox.id).await?;
    let row=sqlx::query_as("SELECT enabled,target_email::text,keep_copy,verified_at,verification_expires_at FROM mail_forwarding WHERE mailbox_id=$1")
      .bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    audit::record(&state,Some(auth.user_id),"mail.forwarding.verify",json!({"target":target})).await;
    Ok(Json(json!({"forwarding":forwarding_json(row),"sync":sync})))
}

pub async fn resend_forwarding(
    State(state):State<AppState>,auth:AuthUser
)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"forwarding").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let row: Option<(String,Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT target_email::text,verification_sent_at FROM mail_forwarding WHERE mailbox_id=$1 AND verified_at IS NULL AND target_email<>''"
    ).bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let (target,last)=row.ok_or_else(||ApiError::bad_request("No forwarding verification is pending"))?;
    if last.is_some_and(|at| chrono::Utc::now()-at < chrono::Duration::seconds(60)) {
        return Err(ApiError::too_many("Wait a minute before requesting another code"));
    }
    let dev_code=issue_forward_code(&state,mailbox.id,&target).await?;
    Ok(Json(json!({"ok":true,"verificationCode":dev_code})))
}

fn vacation_json(row:Option<(bool,String,String,bool,Option<chrono::NaiveDate>,Option<chrono::NaiveDate>)>)->Value{
    match row {
      Some((enabled,subject,message,only_contacts,starts,ends))=>json!({"enabled":enabled,"subject":subject,"message":message,"onlyContacts":only_contacts,"startsAt":starts,"endsAt":ends}),
      None=>json!({"enabled":false,"subject":"Out of office","message":"Thanks for your email. I'm currently out of office and will get back to you as soon as I can.","onlyContacts":true,"startsAt":null,"endsAt":null})
    }
}

pub async fn get_vacation(State(state):State<AppState>,auth:AuthUser)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"vacation").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let row=sqlx::query_as("SELECT enabled,subject,message,only_contacts,starts_at,ends_at FROM mail_vacation WHERE mailbox_id=$1")
      .bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let sync=automation::sync_status(&state,mailbox.id).await.map_err(ApiError::internal)?;
    Ok(Json(json!({"vacation":vacation_json(row),"sync":sync})))
}

#[derive(Deserialize)]
pub struct VacationIn{
    enabled:bool, subject:String, message:String,
    #[serde(rename="onlyContacts")] only_contacts:bool,
    #[serde(rename="startsAt",default)] starts_at:Option<chrono::NaiveDate>,
    #[serde(rename="endsAt",default)] ends_at:Option<chrono::NaiveDate>,
}

pub async fn put_vacation(
    State(state):State<AppState>,auth:AuthUser,Json(body):Json<VacationIn>
)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"vacation").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    let subject=body.subject.trim();
    let message=body.message.trim();
    if subject.is_empty() || subject.chars().count()>200 || subject.chars().any(char::is_control) {
        return Err(ApiError::bad_request("Auto-reply subject must be between 1 and 200 printable characters"));
    }
    if message.is_empty() || message.chars().count()>10000 || message.chars().any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t')) {
        return Err(ApiError::bad_request("Auto-reply message must be between 1 and 10000 characters"));
    }
    if body.starts_at.as_ref().zip(body.ends_at.as_ref()).is_some_and(|(start, end)| end < start) {
        return Err(ApiError::bad_request("Reply end date must not be before the start date"));
    }
    sqlx::query(
        "INSERT INTO mail_vacation(user_id,mailbox_id,enabled,subject,message,only_contacts,starts_at,ends_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT(mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET enabled=EXCLUDED.enabled,subject=EXCLUDED.subject,message=EXCLUDED.message,
          only_contacts=EXCLUDED.only_contacts,starts_at=EXCLUDED.starts_at,ends_at=EXCLUDED.ends_at,updated_at=now()"
    ).bind(auth.user_id).bind(mailbox.id).bind(body.enabled).bind(subject).bind(message).bind(body.only_contacts).bind(body.starts_at).bind(body.ends_at)
      .execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let sync=sync_after_change(&state,auth.user_id,mailbox.id).await?;
    audit::record(&state,Some(auth.user_id),"mail.vacation.update",json!({"enabled":body.enabled})).await;
    let row=sqlx::query_as("SELECT enabled,subject,message,only_contacts,starts_at,ends_at FROM mail_vacation WHERE mailbox_id=$1")
      .bind(mailbox.id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    Ok(Json(json!({"vacation":vacation_json(row),"sync":sync})))
}

pub async fn status(State(state):State<AppState>,auth:AuthUser)->Result<Json<Value>,ApiError>{
    require_feature(&state,auth.user_id,"mail_rules").await?;
    let mailbox = active_mailbox(&state, &auth).await?;
    Ok(Json(automation::sync_status(&state,mailbox.id).await.map_err(ApiError::internal)?))
}
