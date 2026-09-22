use axum::extract::{Path,State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json,Value};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::middleware::auth::AuthUser;
use crate::services::{business_addressing,entitlements,tenancy};
use crate::state::AppState;

fn valid_local(value:&str)->bool{
    let v=value.trim();
    !v.is_empty()&&v.len()<=64&&!v.starts_with('.')&&!v.ends_with('.')&&!v.contains("..")&&
      v.chars().all(|c|c.is_ascii_alphanumeric()||matches!(c,'.'|'-'|'_'))
}

#[derive(Deserialize)]
pub struct AddressIn{
    pub domain_id:Uuid,
    pub local_part:String,
    pub kind:String,
    pub mailbox_ids:Vec<Uuid>,
}

pub async fn list(State(state):State<AppState>,auth:AuthUser,Path(organization_id):Path<Uuid>)->Result<Json<Value>,ApiError>{
    tenancy::require_member(&state.db,auth.user_id,organization_id).await?;
    let rows:Vec<(Uuid,Uuid,String,String,String,bool,String,String,Option<String>)>=sqlx::query_as(
        "SELECT id,domain_id,address::text,local_part,kind,enabled,sync_status,sync_error,provider_object_id
         FROM business_addresses WHERE organization_id=$1 AND deleted_at IS NULL ORDER BY kind,lower(address::text)"
    ).bind(organization_id).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let mut out=Vec::new();
    for row in rows{
        let members:Vec<(Uuid,String)>=sqlx::query_as(
            "SELECT m.id,m.address::text FROM business_address_members bam JOIN mailboxes m ON m.id=bam.mailbox_id
             WHERE bam.business_address_id=$1 AND m.deleted_at IS NULL ORDER BY lower(m.address::text)"
        ).bind(row.0).fetch_all(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
        out.push(json!({"id":row.0,"domain_id":row.1,"address":row.2,"local_part":row.3,"kind":row.4,"enabled":row.5,
          "sync_status":row.6,"sync_error":row.7,"provider_object_id":row.8,
          "mailboxes":members.into_iter().map(|m|json!({"id":m.0,"address":m.1})).collect::<Vec<_>>() }));
    }
    Ok(Json(json!({"addresses":out})))
}

pub async fn create(State(state):State<AppState>,auth:AuthUser,Path(organization_id):Path<Uuid>,Json(body):Json<AddressIn>)
 ->Result<(axum::http::StatusCode,Json<Value>),ApiError>{
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let local=body.local_part.trim().to_ascii_lowercase();
    if !valid_local(&local){return Err(ApiError::bad_request("Invalid address local part"));}
    if !matches!(body.kind.as_str(),"alias"|"group"){return Err(ApiError::bad_request("Address kind must be alias or group"));}
    if body.mailbox_ids.is_empty(){return Err(ApiError::bad_request("Choose at least one destination mailbox"));}
    if body.kind=="alias"&&body.mailbox_ids.len()!=1{return Err(ApiError::bad_request("Alias requires exactly one destination mailbox"));}
    let domain:Option<(String,String,Option<String>)>=sqlx::query_as(
        "SELECT domain::text,status,provider_domain_id FROM organization_domains WHERE id=$1 AND organization_id=$2"
    ).bind(body.domain_id).bind(organization_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let (domain_name,status,provider)=domain.ok_or_else(||ApiError::not_found("Business domain not found"))?;
    if status!="active"||provider.as_deref().unwrap_or("").is_empty(){return Err(ApiError::conflict("Aliases/groups require an active mail domain"));}
    let count:i64=sqlx::query_scalar(
        "SELECT COUNT(*) FROM mailboxes WHERE organization_id=$1 AND id=ANY($2) AND status='active' AND deleted_at IS NULL"
    ).bind(organization_id).bind(&body.mailbox_ids).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    if count!=body.mailbox_ids.len() as i64{return Err(ApiError::bad_request("Every destination must be an active mailbox in this business"));}
    if body.kind=="alias" {
        let ent=entitlements::for_organization(&state,organization_id).await?;
        if let Some(limit)=ent.plan.alias_limit_per_mailbox {
            let mailbox_id=body.mailbox_ids[0];
            let used:i64=sqlx::query_scalar(
                "SELECT count(*) FROM business_addresses ba
                 JOIN business_address_members bam ON bam.business_address_id=ba.id
                 WHERE ba.organization_id=$1 AND ba.kind='alias' AND ba.deleted_at IS NULL AND bam.mailbox_id=$2"
            ).bind(organization_id).bind(mailbox_id).fetch_one(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
            if used>=limit as i64 {
                return Err(ApiError::forbidden(format!("This mailbox has reached its alias limit ({limit}) for the current plan")));
            }
        }
    }
    let id=Uuid::new_v4();
    let address=format!("{local}@{domain_name}");
    let marker=state.stalwart.ownership_marker("address", &organization_id.to_string(), &id.to_string());
    let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
    sqlx::query(
        "INSERT INTO business_addresses(id,organization_id,domain_id,local_part,address,kind,provider_marker,created_by)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8)"
    ).bind(id).bind(organization_id).bind(body.domain_id).bind(&local).bind(&address).bind(&body.kind).bind(&marker).bind(auth.user_id)
      .execute(&mut *tx).await.map_err(|e|{if let sqlx::Error::Database(db)=&e{if db.code().as_deref()==Some("23505"){return ApiError::conflict("That business address is already in use");}}ApiError::internal(e.to_string())})?;
    for mailbox_id in &body.mailbox_ids{
        sqlx::query("INSERT INTO business_address_members(business_address_id,mailbox_id) VALUES($1,$2)")
            .bind(id).bind(mailbox_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    }
    tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let sync=business_addressing::sync_after_change(&state,id).await;
    audit::record(&state,Some(auth.user_id),"business.address.create",json!({"organization_id":organization_id,"id":id,"address":address,"kind":body.kind})).await;
    Ok((axum::http::StatusCode::CREATED,Json(json!({"id":id,"address":address,"kind":body.kind,"sync":sync}))))
}

#[derive(Deserialize)]
pub struct UpdateAddressIn{pub enabled:Option<bool>,pub mailbox_ids:Option<Vec<Uuid>>}

pub async fn update(State(state):State<AppState>,auth:AuthUser,Path((organization_id,address_id)):Path<(Uuid,Uuid)>,Json(body):Json<UpdateAddressIn>)->Result<Json<Value>,ApiError>{
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let current:Option<String>=sqlx::query_scalar("SELECT kind FROM business_addresses WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL")
        .bind(address_id).bind(organization_id).fetch_optional(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?;
    let kind=current.ok_or_else(||ApiError::not_found("Business alias/group not found"))?;
    let mut tx=state.db.begin().await.map_err(|e|ApiError::internal(e.to_string()))?;
    if let Some(ids)=body.mailbox_ids.as_ref(){
        if ids.is_empty()||(kind=="alias"&&ids.len()!=1){return Err(ApiError::bad_request(if kind=="alias"{"Alias requires exactly one destination mailbox"}else{"Group requires at least one destination mailbox"}));}
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM mailboxes WHERE organization_id=$1 AND id=ANY($2) AND status='active' AND deleted_at IS NULL")
          .bind(organization_id).bind(ids).fetch_one(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
        if count!=ids.len() as i64{return Err(ApiError::bad_request("Every destination must be an active mailbox in this business"));}
        if kind=="alias" {
            let ent=entitlements::for_organization(&state,organization_id).await?;
            if let Some(limit)=ent.plan.alias_limit_per_mailbox {
                let used:i64=sqlx::query_scalar(
                    "SELECT count(*) FROM business_addresses ba
                     JOIN business_address_members bam ON bam.business_address_id=ba.id
                     WHERE ba.organization_id=$1 AND ba.kind='alias' AND ba.deleted_at IS NULL
                       AND bam.mailbox_id=$2 AND ba.id<>$3"
                ).bind(organization_id).bind(ids[0]).bind(address_id).fetch_one(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
                if used>=limit as i64 {return Err(ApiError::forbidden(format!("This mailbox has reached its alias limit ({limit}) for the current plan")));}
            }
        }
        sqlx::query("DELETE FROM business_address_members WHERE business_address_id=$1").bind(address_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
        for mailbox_id in ids{sqlx::query("INSERT INTO business_address_members(business_address_id,mailbox_id) VALUES($1,$2)").bind(address_id).bind(mailbox_id).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;}
    }
    sqlx::query("UPDATE business_addresses SET enabled=COALESCE($3,enabled),sync_status='pending',sync_error='',next_attempt_at=now(),updated_at=now() WHERE id=$1 AND organization_id=$2")
      .bind(address_id).bind(organization_id).bind(body.enabled).execute(&mut *tx).await.map_err(|e|ApiError::internal(e.to_string()))?;
    tx.commit().await.map_err(|e|ApiError::internal(e.to_string()))?;
    let sync=business_addressing::sync_after_change(&state,address_id).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}

pub async fn delete(State(state):State<AppState>,auth:AuthUser,Path((organization_id,address_id)):Path<(Uuid,Uuid)>)->Result<Json<Value>,ApiError>{
    tenancy::require_admin(&state.db,auth.user_id,organization_id).await?;
    let changed=sqlx::query("UPDATE business_addresses SET deleted_at=now(),enabled=FALSE,sync_status='pending',sync_error='',next_attempt_at=now(),updated_at=now() WHERE id=$1 AND organization_id=$2 AND deleted_at IS NULL")
      .bind(address_id).bind(organization_id).execute(&state.db).await.map_err(|e|ApiError::internal(e.to_string()))?.rows_affected();
    if changed==0{return Err(ApiError::not_found("Business alias/group not found"));}
    let sync=business_addressing::sync_after_change(&state,address_id).await;
    audit::record(&state,Some(auth.user_id),"business.address.delete",json!({"organization_id":organization_id,"id":address_id})).await;
    Ok(Json(json!({"ok":true,"sync":sync})))
}
