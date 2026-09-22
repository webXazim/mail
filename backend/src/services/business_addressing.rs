use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::state::AppState;

fn safe_error(value: &str) -> String { value.chars().take(800).collect() }

pub async fn sync_one(state: &AppState, address_id: Uuid) -> Result<Value, String> {
    let mut conn = state.db.acquire().await.map_err(|e| e.to_string())?;
    let lock_key = format!("business-address:{address_id}");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1,0))")
        .bind(&lock_key).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    let result = sync_one_locked(state,address_id).await;
    let _=sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1,0))").bind(&lock_key).execute(&mut *conn).await;
    result
}

async fn sync_one_locked(state:&AppState,address_id:Uuid)->Result<Value,String>{
    let row: Option<(Uuid,Uuid,String,String,String,bool,String,Option<String>,Option<chrono::DateTime<chrono::Utc>>,Option<String>,String)> = sqlx::query_as(
        "UPDATE business_addresses a SET sync_status='syncing',updated_at=now()
         WHERE a.id=$1 AND a.sync_status IN ('pending','error')
         RETURNING a.organization_id,a.domain_id,a.local_part,a.address::text,a.kind,a.enabled,a.provider_marker,
                   a.provider_object_id,a.deleted_at,
                   (SELECT d.provider_domain_id FROM organization_domains d WHERE d.id=a.domain_id),
                   (SELECT d.status FROM organization_domains d WHERE d.id=a.domain_id)",
    ).bind(address_id).fetch_optional(&state.db).await.map_err(|e|e.to_string())?;
    let Some((organization_id,_domain_id,local,_address,kind,enabled,marker,provider_object_id,deleted_at,provider_domain_id,domain_status))=row else {
        return Ok(json!({"status":"ready"}));
    };
    if deleted_at.is_some(){
        let provider_domain_id = provider_domain_id.filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Business domain provider binding is missing; refusing provider deletion".to_string())?;
        state.stalwart.delete_business_mailing_list(&provider_domain_id,provider_object_id.as_deref(),&marker,&local).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE business_addresses SET provider_object_id=NULL,sync_status='deleted',sync_error='',synced_at=now(),updated_at=now() WHERE id=$1")
            .bind(address_id).execute(&state.db).await.map_err(|e|e.to_string())?;
        return Ok(json!({"status":"deleted"}));
    }
    if domain_status!="active" { return Err("Business domain is not active".into()); }
    let provider_domain_id = provider_domain_id.filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Business domain provider binding is missing".to_string())?;
    let recipients:Vec<String>=sqlx::query_scalar(
        "SELECT m.address::text FROM business_address_members bam JOIN mailboxes m ON m.id=bam.mailbox_id
         WHERE bam.business_address_id=$1 AND m.organization_id=$2 AND m.deleted_at IS NULL AND m.status='active'
         ORDER BY lower(m.address::text)",
    ).bind(address_id).bind(organization_id).fetch_all(&state.db).await.map_err(|e|e.to_string())?;
    if recipients.is_empty(){ return Err("Alias/group has no active destination mailboxes".into()); }
    if kind=="alias" && recipients.len()!=1 { return Err("Alias must have exactly one destination mailbox".into()); }
    let provider_id=state.stalwart.ensure_business_mailing_list(
        &provider_domain_id,provider_object_id.as_deref(),&marker,&local,&recipients,enabled
    ).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE business_addresses SET provider_object_id=$2,sync_status='ready',sync_error='',sync_attempts=0,next_attempt_at=now(),synced_at=now(),updated_at=now() WHERE id=$1")
        .bind(address_id).bind(provider_id).execute(&state.db).await.map_err(|e|e.to_string())?;
    Ok(json!({"status":"ready","recipients":recipients}))
}

async fn mark_error(state:&AppState,id:Uuid,error:&str){
    let safe=safe_error(error);
    let _=sqlx::query(
        "UPDATE business_addresses SET sync_status='error',sync_error=$2,sync_attempts=sync_attempts+1,
         next_attempt_at=now()+(LEAST(3600,5*power(2,LEAST(sync_attempts,9))::int)*interval '1 second'),updated_at=now()
         WHERE id=$1"
    ).bind(id).bind(safe).execute(&state.db).await;
}

pub async fn sync_after_change(state:&AppState,id:Uuid)->Value{
    match sync_one(state,id).await { Ok(v)=>v, Err(e)=>{ mark_error(state,id,&e).await; json!({"status":"error","last_error":e}) } }
}

pub fn spawn_worker(state:AppState){
    tokio::spawn(async move{
        let mut tick=tokio::time::interval(Duration::from_secs(30));
        loop{
            tick.tick().await;
            let ids:Vec<Uuid>=match sqlx::query_scalar(
                "SELECT id FROM business_addresses WHERE sync_status IN ('pending','error') AND next_attempt_at<=now()
                 ORDER BY next_attempt_at,created_at LIMIT 25"
            ).fetch_all(&state.db).await { Ok(v)=>v, Err(e)=>{tracing::warn!(%e,"business address reconciliation scan failed");continue;} };
            for id in ids { if let Err(e)=sync_one(&state,id).await { mark_error(&state,id,&e).await; tracing::warn!(address_id=%id,%e,"business address reconciliation failed"); } }
        }
    });
}
