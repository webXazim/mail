//! Atomic mailbox + organization + domain send budgets (Upgrade 25).
//! Daily commercial limits are combined with finite hourly burst controls,
//! mailbox/domain warm-up, and operator deliverability restrictions.

use chrono::{DateTime, Timelike, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::send_limit as policy;
use crate::error::ApiError;
use crate::state::AppState;

async fn mailbox_age_days_tx(
    tx: &mut Transaction<'_, Postgres>,
    mailbox_id: Uuid,
    now: DateTime<Utc>,
) -> Result<i64, ApiError> {
    let created: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT created_at FROM mailboxes WHERE id=$1")
            .bind(mailbox_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(created.map_or(0, |created| (now - created).num_days()))
}

async fn domain_age_days_tx(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    domain: &str,
    now: DateTime<Utc>,
) -> Result<i64, ApiError> {
    let activated: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT COALESCE(activated_at,verified_at,created_at)
         FROM organization_domains
         WHERE organization_id=$1 AND lower(domain::text)=lower($2)
           AND status='active' AND dns_ready=TRUE",
    )
    .bind(organization_id)
    .bind(domain)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(activated.map_or(0, |created| (now - created).num_days()))
}

fn hour_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now.with_minute(0)
        .and_then(|value| value.with_second(0))
        .and_then(|value| value.with_nanosecond(0))
        .unwrap_or(now)
}

pub async fn enforce_once(
    state: &AppState,
    request_id: Uuid,
    user_id: Uuid,
    domain: &str,
    mailbox_daily_limit: i64,
    organization_daily_limit: i64,
    recipients: i64,
) -> Result<(), ApiError> {
    if recipients <= 0 || domain.is_empty() {
        return Ok(());
    }

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let row: Option<(bool, Option<Uuid>)> = sqlx::query_as(
        "SELECT budget_charged,mailbox_id
         FROM mail_send_requests
         WHERE id=$1 AND user_id=$2
         FOR UPDATE",
    )
    .bind(request_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some((already_charged, mailbox_id)) = row else {
        return Err(ApiError::internal(
            "Send request disappeared before rate accounting",
        ));
    };
    if already_charged {
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok(());
    }
    let mailbox_id =
        mailbox_id.ok_or_else(|| ApiError::forbidden("A concrete mailbox is required for sending"))?;
    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT organization_id
         FROM mailboxes
         WHERE id=$1 AND deleted_at IS NULL AND status='active'",
    )
    .bind(mailbox_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let control: Option<(String, Option<i32>, Option<i32>)> = sqlx::query_as(
        "SELECT state,hourly_limit_override,daily_limit_override
         FROM organization_sending_controls
         WHERE organization_id=$1
         FOR UPDATE",
    )
    .bind(organization_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let (sending_state, hourly_override, daily_override) =
        control.unwrap_or_else(|| ("active".to_string(), None, None));
    if sending_state == "suspended" {
        return Err(ApiError::forbidden(
            "Outbound sending is suspended for this business. Contact CS Mail support.",
        ));
    }

    let now = Utc::now();
    let day = policy::day_of(now);
    let hour = hour_start(now);
    let grant = recipients;
    let mailbox_age = mailbox_age_days_tx(&mut tx, mailbox_id, now).await?;
    let domain_age = domain_age_days_tx(&mut tx, organization_id, domain, now).await?;

    let mailbox_daily = policy::effective_daily(mailbox_daily_limit, mailbox_age);
    let domain_warmup = policy::warmup_daily(domain_age);
    let domain_daily = if domain_warmup > 0 {
        policy::PER_DOMAIN_DAILY.min(domain_warmup.saturating_mul(10))
    } else {
        policy::PER_DOMAIN_DAILY
    };

    let mut organization_daily =
        policy::capped_limit(organization_daily_limit, daily_override);
    let mut organization_hourly = hourly_override
        .map(i64::from)
        .filter(|value| *value > 0)
        .unwrap_or_else(|| policy::organization_hourly(organization_daily));
    if sending_state == "restricted" {
        organization_daily = if organization_daily <= 0 {
            policy::RESTRICTED_ORG_DAILY
        } else {
            organization_daily.min(policy::RESTRICTED_ORG_DAILY)
        };
        organization_hourly = organization_hourly.min(policy::RESTRICTED_ORG_HOURLY);
    }
    let mailbox_hourly = policy::mailbox_hourly(mailbox_daily_limit, mailbox_age);

    let mailbox_count: i64 = sqlx::query_scalar(
        "INSERT INTO mailbox_send_counters(mailbox_id,day,sent_count) VALUES($1,$2,$3)
         ON CONFLICT(mailbox_id,day) DO UPDATE
           SET sent_count=mailbox_send_counters.sent_count+EXCLUDED.sent_count,updated_at=now()
         RETURNING sent_count",
    )
    .bind(mailbox_id)
    .bind(day)
    .bind(grant)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let org_count: i64 = sqlx::query_scalar(
        "INSERT INTO organization_send_counters(organization_id,day,sent_count) VALUES($1,$2,$3)
         ON CONFLICT(organization_id,day) DO UPDATE
           SET sent_count=organization_send_counters.sent_count+EXCLUDED.sent_count,updated_at=now()
         RETURNING sent_count",
    )
    .bind(organization_id)
    .bind(day)
    .bind(grant)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let domain_count: i32 = sqlx::query_scalar(
        "INSERT INTO send_domain_counters(domain,day,count) VALUES($1,$2,$3)
         ON CONFLICT(domain,day) DO UPDATE
           SET count=send_domain_counters.count+EXCLUDED.count
         RETURNING count",
    )
    .bind(domain)
    .bind(day)
    .bind(grant as i32)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let mailbox_hour_count: i64 = sqlx::query_scalar(
        "INSERT INTO mailbox_send_hourly_counters(mailbox_id,hour_start,sent_count) VALUES($1,$2,$3)
         ON CONFLICT(mailbox_id,hour_start) DO UPDATE
           SET sent_count=mailbox_send_hourly_counters.sent_count+EXCLUDED.sent_count,updated_at=now()
         RETURNING sent_count",
    )
    .bind(mailbox_id)
    .bind(hour)
    .bind(grant)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let org_hour_count: i64 = sqlx::query_scalar(
        "INSERT INTO organization_send_hourly_counters(organization_id,hour_start,sent_count) VALUES($1,$2,$3)
         ON CONFLICT(organization_id,hour_start) DO UPDATE
           SET sent_count=organization_send_hourly_counters.sent_count+EXCLUDED.sent_count,updated_at=now()
         RETURNING sent_count",
    )
    .bind(organization_id)
    .bind(hour)
    .bind(grant)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let domain_hour_count: i64 = sqlx::query_scalar(
        "INSERT INTO domain_send_hourly_counters(domain,hour_start,sent_count) VALUES($1,$2,$3)
         ON CONFLICT(domain,hour_start) DO UPDATE
           SET sent_count=domain_send_hourly_counters.sent_count+EXCLUDED.sent_count,updated_at=now()
         RETURNING sent_count",
    )
    .bind(domain)
    .bind(hour)
    .bind(grant)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    if policy::exceeded(mailbox_count, mailbox_daily) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(format!(
            "This mailbox has reached its daily sending limit ({mailbox_daily} recipients). Try again after 00:00 UTC."
        )));
    }
    if organization_daily > 0 && policy::exceeded(org_count, organization_daily) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(format!(
            "This business has reached its daily sending limit ({organization_daily} recipients)."
        )));
    }
    if policy::exceeded(domain_count as i64, domain_daily) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(
            "This sending domain has reached its daily reputation ceiling.",
        ));
    }
    if policy::exceeded(mailbox_hour_count, mailbox_hourly) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(format!(
            "This mailbox has reached its hourly sending burst limit ({mailbox_hourly} recipients)."
        )));
    }
    if policy::exceeded(org_hour_count, organization_hourly) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(format!(
            "This business has reached its hourly sending burst limit ({organization_hourly} recipients)."
        )));
    }
    if policy::exceeded(domain_hour_count, policy::PER_DOMAIN_HOURLY) {
        state.metrics.record_rate_limited();
        return Err(ApiError::too_many(
            "This sending domain has reached its hourly reputation ceiling.",
        ));
    }

    sqlx::query(
        "UPDATE mail_send_requests SET budget_charged=TRUE,updated_at=now()
         WHERE id=$1 AND user_id=$2",
    )
    .bind(request_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}
