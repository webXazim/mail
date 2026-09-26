//! Durable PostgreSQL -> Stalwart mutation queue.
//!
//! Provider writes must survive API restarts and temporary mail-server
//! outages.  Jobs are claimed with `FOR UPDATE SKIP LOCKED`, leased, retried
//! with bounded exponential backoff, and reconciled against the authoritative
//! organization/mailbox row before they mutate Stalwart. Passwords needed to create a mailbox are
//! encrypted at rest with PostgreSQL pgcrypto (AES-256) and are erased as soon
//! as the job reaches a terminal state.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::ApiError;
use crate::services::stalwart::StalwartError;
use crate::state::AppState;

const OP_ENSURE: &str = "ensure_mailbox";
const OP_QUOTA: &str = "set_quota";
const OP_CREDENTIALS: &str = "set_credentials";
const OP_ACCESS: &str = "set_access";
const OP_DELETE: &str = "delete_mailbox";
const MAX_ERROR_CHARS: usize = 800;

#[derive(Clone)]
pub struct ProvisioningService {
    inner: Arc<ProvisioningConfig>,
}

struct ProvisioningConfig {
    key: String,
    poll_interval: Duration,
    lease_timeout: Duration,
    retry_base: Duration,
    reconcile_interval: Duration,
    max_attempts: i32,
    batch_size: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct MailboxTarget {
    mailbox_id: Uuid,
    organization_id: Uuid,
    address: String,
    provider_account_id: Option<String>,
}

async fn primary_mailbox_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Option<MailboxTarget>, ApiError> {
    sqlx::query_as::<_, MailboxTarget>(
        "SELECT m.id AS mailbox_id, m.organization_id, m.address::text AS address,
                m.provider_account_id
         FROM users u
         JOIN mailboxes m ON m.id = u.primary_mailbox_id AND m.deleted_at IS NULL
         JOIN organizations o ON o.id = m.organization_id
         WHERE u.id = $1 AND o.status <> 'closed' AND m.status <> 'deleting'",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))
}

impl ProvisioningService {
    pub fn new(
        key: String,
        poll_interval: Duration,
        lease_timeout: Duration,
        retry_base: Duration,
        reconcile_interval: Duration,
        max_attempts: i32,
        batch_size: i64,
    ) -> Result<Self, String> {
        if key.len() < 32 {
            return Err("provisioning encryption key must contain at least 32 characters".into());
        }
        if poll_interval.is_zero() {
            return Err("provisioning poll interval must be greater than zero".into());
        }
        if lease_timeout < Duration::from_secs(15) {
            return Err("provisioning lease timeout must be at least 15 seconds".into());
        }
        if retry_base.is_zero() {
            return Err("provisioning retry base must be greater than zero".into());
        }
        if reconcile_interval < Duration::from_secs(30) {
            return Err("provisioning reconcile interval must be at least 30 seconds".into());
        }
        if max_attempts < 1 {
            return Err("provisioning max attempts must be at least 1".into());
        }
        if !(1..=100).contains(&batch_size) {
            return Err("provisioning batch size must be between 1 and 100".into());
        }
        Ok(Self {
            inner: Arc::new(ProvisioningConfig {
                key,
                poll_interval,
                lease_timeout,
                retry_base,
                reconcile_interval,
                max_attempts,
                batch_size,
            }),
        })
    }

    fn secret_payload(user_id: Uuid, mailbox_id: Uuid, email: &str, secret: &str) -> String {
        format!(
            "{user_id}\n{mailbox_id}\n{}\n{secret}",
            email.trim().to_lowercase()
        )
    }

    async fn decrypt_secret(&self, pool: &PgPool, job: &JobRow) -> Result<String, JobFailure> {
        let user_id = job
            .user_id
            .ok_or_else(|| JobFailure::permanent("Provisioning user no longer exists"))?;
        if !job.has_secret {
            return Err(JobFailure::permanent("Provisioning credential is missing"));
        }
        let protected: Option<String> = sqlx::query_scalar(
            "SELECT pgp_sym_decrypt(secret_ciphertext, $2)
             FROM provisioning_jobs WHERE id = $1",
        )
        .bind(job.id)
        .bind(&self.inner.key)
        .fetch_optional(pool)
        .await
        .map_err(|e| JobFailure::permanent(format!("Provisioning credential decrypt failed: {e}")))?;
        let protected = protected
            .ok_or_else(|| JobFailure::permanent("Provisioning credential is missing"))?;
        let parts = protected.split('\n').collect::<Vec<_>>();
        let (stored_user, stored_mailbox, stored_email, secret) = match parts.as_slice() {
            // Upgrade 19 mailbox-bound payload.
            [stored_user, stored_mailbox, stored_email, secret] => {
                (*stored_user, Some(*stored_mailbox), *stored_email, *secret)
            }
            // Backward-compatible reader for jobs encrypted before Upgrade 19.
            [stored_user, stored_email, secret] => (*stored_user, None, *stored_email, *secret),
            _ => {
                return Err(JobFailure::permanent(
                    "Provisioning credential identity binding is invalid",
                ))
            }
        };
        let mailbox_matches = match (stored_mailbox, job.mailbox_id) {
            (Some(value), Some(mailbox_id)) => value == mailbox_id.to_string(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        if stored_user != user_id.to_string()
            || !mailbox_matches
            || !stored_email.eq_ignore_ascii_case(&job.target_email)
            || secret.is_empty()
        {
            return Err(JobFailure::permanent(
                "Provisioning credential identity binding is invalid",
            ));
        }
        Ok(secret.to_string())
    }

    /// Queue mailbox creation only after a first-class hosted mailbox row exists.
    /// The caller-supplied login email is intentionally ignored; provider authority
    /// comes from `users.primary_mailbox_id -> mailboxes`.
    pub async fn enqueue_ensure_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        _email: &str,
        password: &str,
        quota_bytes: i64,
    ) -> Result<(), ApiError> {
        let target = primary_mailbox_tx(tx, user_id)
            .await?
            .ok_or_else(|| ApiError::bad_request("User does not have a hosted mailbox"))?;
        let protected = Self::secret_payload(user_id, target.mailbox_id, &target.address, password);
        let dedupe = format!("mailbox.ensure:{}", target.mailbox_id);
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, quota_bytes, secret_ciphertext,
                 status, max_attempts, dedupe_key)
             VALUES ($1, $2, $3, $4, $5, NULLIF($6, ''), $7,
                     pgp_sym_encrypt($8, $9, 'cipher-algo=aes256, compress-algo=0'),
                     'pending', $10, $11)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id = EXCLUDED.user_id, organization_id = EXCLUDED.organization_id,
                           mailbox_id = EXCLUDED.mailbox_id, target_email = EXCLUDED.target_email,
                           account_id = COALESCE(EXCLUDED.account_id, provisioning_jobs.account_id),
                           quota_bytes = EXCLUDED.quota_bytes, secret_ciphertext = EXCLUDED.secret_ciphertext,
                           status = 'pending', attempts = 0, max_attempts = EXCLUDED.max_attempts,
                           next_attempt_at = now(), last_error = '', last_failure_transient = FALSE,
                           completed_at = NULL, updated_at = now()",
        )
        .bind(user_id)
        .bind(target.organization_id)
        .bind(target.mailbox_id)
        .bind(OP_ENSURE)
        .bind(&target.address)
        .bind(target.provider_account_id.as_deref().unwrap_or(""))
        .bind(quota_bytes)
        .bind(protected)
        .bind(&self.inner.key)
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        sqlx::query(
            "UPDATE mailboxes SET sync_status='pending', sync_error='', quota_bytes=GREATEST($2,1048576), updated_at=now()
             WHERE id=$1",
        )
        .bind(target.mailbox_id)
        .bind(quota_bytes)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query(
            "UPDATE users SET mail_sync_status = 'pending', mail_sync_error = '', updated_at = now()
             WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    /// Queue a newly assigned business mailbox. Unlike the legacy user helper,
    /// this entry point is explicit about the mailbox id so organization admins
    /// cannot accidentally target a platform login address.
    pub async fn enqueue_mailbox_ensure_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mailbox_id: Uuid,
        user_id: Uuid,
        password: &str,
    ) -> Result<(), ApiError> {
        let target: Option<(Uuid, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT organization_id, address::text, provider_account_id, quota_bytes
             FROM mailboxes
             WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL AND status IN ('pending','active','suspended')",
        )
        .bind(mailbox_id)
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let (organization_id, address, account_id, quota_bytes) = target
            .ok_or_else(|| ApiError::bad_request("Hosted mailbox is not assigned to this user"))?;
        let protected = Self::secret_payload(user_id, mailbox_id, &address, password);
        let dedupe = format!("mailbox.ensure:{mailbox_id}");
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, quota_bytes, secret_ciphertext,
                 status, max_attempts, dedupe_key)
             VALUES ($1,$2,$3,$4,$5,NULLIF($6,''),$7,
                     pgp_sym_encrypt($8,$9,'cipher-algo=aes256, compress-algo=0'),
                     'pending',$10,$11)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id=EXCLUDED.user_id, organization_id=EXCLUDED.organization_id,
                 mailbox_id=EXCLUDED.mailbox_id,target_email=EXCLUDED.target_email,
                 account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),quota_bytes=EXCLUDED.quota_bytes,
                 secret_ciphertext=EXCLUDED.secret_ciphertext,status='pending',attempts=0,max_attempts=EXCLUDED.max_attempts,
                 next_attempt_at=now(),last_error='',last_failure_transient=FALSE,completed_at=NULL,updated_at=now()"
        )
        .bind(user_id).bind(organization_id).bind(mailbox_id).bind(OP_ENSURE).bind(&address)
        .bind(account_id.as_deref().unwrap_or("")).bind(quota_bytes).bind(protected).bind(&self.inner.key)
        .bind(self.inner.max_attempts).bind(dedupe)
        .execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET sync_status='pending',sync_error='',updated_at=now() WHERE id=$1")
            .bind(mailbox_id).execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE users SET mail_sync_status='pending',mail_sync_error='',updated_at=now() WHERE id=$1")
            .bind(user_id).execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn enqueue_mailbox_access_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mailbox_id: Uuid,
    ) -> Result<(), ApiError> {
        let target: Option<(Uuid, Option<Uuid>, String, Option<String>)> = sqlx::query_as(
            "SELECT organization_id,user_id,address::text,provider_account_id FROM mailboxes
             WHERE id=$1 AND deleted_at IS NULL AND status IN ('pending','active','suspended')",
        ).bind(mailbox_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        let (organization_id,user_id,address,account_id) = target.ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
        let dedupe=format!("mailbox.access:{mailbox_id}");
        sqlx::query(
            "INSERT INTO provisioning_jobs(user_id,organization_id,mailbox_id,operation,target_email,account_id,status,max_attempts,dedupe_key)
             VALUES($1,$2,$3,$4,$5,NULLIF($6,''),'pending',$7,$8)
             ON CONFLICT(dedupe_key) WHERE status IN ('pending','retry') DO UPDATE SET
               user_id=EXCLUDED.user_id,organization_id=EXCLUDED.organization_id,mailbox_id=EXCLUDED.mailbox_id,
               target_email=EXCLUDED.target_email,account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),
               status='pending',attempts=0,max_attempts=EXCLUDED.max_attempts,next_attempt_at=now(),last_error='',
               last_failure_transient=FALSE,completed_at=NULL,updated_at=now()"
        ).bind(user_id).bind(organization_id).bind(mailbox_id).bind(OP_ACCESS).bind(&address)
         .bind(account_id.as_deref().unwrap_or("")).bind(self.inner.max_attempts).bind(dedupe)
         .execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET sync_status='retrying',sync_error='',updated_at=now() WHERE id=$1")
          .bind(mailbox_id).execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn enqueue_mailbox_delete_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mailbox_id: Uuid,
    ) -> Result<(), ApiError> {
        let target: Option<(Uuid, Option<Uuid>, String, Option<String>)> = sqlx::query_as(
            "SELECT organization_id,user_id,address::text,provider_account_id FROM mailboxes WHERE id=$1 AND deleted_at IS NULL",
        ).bind(mailbox_id).fetch_optional(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        let (organization_id,user_id,address,account_id)=target.ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
        let dedupe=format!("mailbox.delete:{mailbox_id}");
        sqlx::query(
            "INSERT INTO provisioning_jobs(user_id,organization_id,mailbox_id,operation,target_email,account_id,status,max_attempts,dedupe_key)
             VALUES($1,$2,$3,$4,$5,NULLIF($6,''),'pending',$7,$8)
             ON CONFLICT(dedupe_key) WHERE status IN ('pending','retry') DO UPDATE SET
               user_id=EXCLUDED.user_id,organization_id=EXCLUDED.organization_id,mailbox_id=EXCLUDED.mailbox_id,
               target_email=EXCLUDED.target_email,account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),
               status='pending',attempts=0,max_attempts=EXCLUDED.max_attempts,next_attempt_at=now(),last_error='',
               last_failure_transient=FALSE,completed_at=NULL,updated_at=now()"
        ).bind(user_id).bind(organization_id).bind(mailbox_id).bind(OP_DELETE).bind(&address)
         .bind(account_id.as_deref().unwrap_or("")).bind(self.inner.max_attempts).bind(dedupe)
         .execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET status='deleting',sync_status='pending',sync_error='',updated_at=now() WHERE id=$1 AND deleted_at IS NULL")
            .bind(mailbox_id).execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn enqueue_quota(
        &self,
        pool: &PgPool,
        user_id: Uuid,
        email: &str,
        account_id: Option<&str>,
        quota_bytes: i64,
    ) -> Result<(), ApiError> {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        self.enqueue_quota_tx(&mut tx, user_id, email, account_id, quota_bytes)
            .await?;
        tx.commit()
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn enqueue_quota_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        _email: &str,
        _account_id: Option<&str>,
        quota_bytes: i64,
    ) -> Result<(), ApiError> {
        let Some(target) = primary_mailbox_tx(tx, user_id).await? else {
            return Ok(());
        };
        let dedupe = format!("mailbox.quota:{}", target.mailbox_id);
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, quota_bytes, status, max_attempts, dedupe_key)
             VALUES ($1, $2, $3, $4, $5, NULLIF($6, ''), $7, 'pending', $8, $9)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id=EXCLUDED.user_id, organization_id=EXCLUDED.organization_id,
                           mailbox_id=EXCLUDED.mailbox_id, target_email=EXCLUDED.target_email,
                           account_id=COALESCE(EXCLUDED.account_id, provisioning_jobs.account_id),
                           quota_bytes=EXCLUDED.quota_bytes, status='pending', attempts=0,
                           max_attempts=EXCLUDED.max_attempts, next_attempt_at=now(), last_error='',
                           last_failure_transient=FALSE, completed_at=NULL, updated_at=now()",
        )
        .bind(user_id)
        .bind(target.organization_id)
        .bind(target.mailbox_id)
        .bind(OP_QUOTA)
        .bind(&target.address)
        .bind(target.provider_account_id.as_deref().unwrap_or(""))
        .bind(quota_bytes)
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        sqlx::query(
            "UPDATE mailboxes SET quota_bytes=GREATEST($2,1048576),
             sync_status=CASE WHEN COALESCE(provider_account_id,'')='' THEN 'pending' ELSE 'retrying' END,
             sync_error='', updated_at=now() WHERE id=$1",
        )
        .bind(target.mailbox_id)
        .bind(quota_bytes)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query(
            "UPDATE users SET mail_sync_status=CASE WHEN COALESCE(mail_account_id,'')='' THEN 'pending' ELSE 'retrying' END,
             mail_sync_error='', quota_bytes=$2, updated_at=now() WHERE id=$1",
        )
        .bind(user_id)
        .bind(quota_bytes)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    /// Queue a quota update for one specific hosted mailbox. Unlike the legacy
    /// user-scoped helper this does not assume the user's primary mailbox, so a
    /// business admin can safely allocate different storage to several hosted
    /// addresses owned by the same platform identity.
    pub async fn enqueue_mailbox_quota_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mailbox_id: Uuid,
        quota_bytes: i64,
    ) -> Result<(), ApiError> {
        let row: Option<(Option<Uuid>, Option<Uuid>, Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT m.user_id,o.created_by,m.organization_id,m.address::text,m.provider_account_id
             FROM mailboxes m JOIN organizations o ON o.id=m.organization_id
             WHERE m.id=$1 AND m.deleted_at IS NULL"
        )
        .bind(mailbox_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let (mailbox_user,created_by,organization_id,address,account_id)=row
            .ok_or_else(|| ApiError::not_found("Mailbox not found"))?;
        let job_user=mailbox_user.or(created_by);
        let dedupe=format!("mailbox.quota:{mailbox_id}");
        sqlx::query(
            "INSERT INTO provisioning_jobs
              (user_id,organization_id,mailbox_id,operation,target_email,account_id,quota_bytes,status,max_attempts,dedupe_key)
             VALUES($1,$2,$3,$4,$5,NULLIF($6,''),$7,'pending',$8,$9)
             ON CONFLICT(dedupe_key) WHERE status IN ('pending','retry') DO UPDATE SET
               user_id=EXCLUDED.user_id,organization_id=EXCLUDED.organization_id,mailbox_id=EXCLUDED.mailbox_id,
               target_email=EXCLUDED.target_email,account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),
               quota_bytes=EXCLUDED.quota_bytes,status='pending',attempts=0,max_attempts=EXCLUDED.max_attempts,
               next_attempt_at=now(),last_error='',last_failure_transient=FALSE,completed_at=NULL,updated_at=now()"
        )
        .bind(job_user)
        .bind(organization_id)
        .bind(mailbox_id)
        .bind(OP_QUOTA)
        .bind(&address)
        .bind(account_id.as_deref().unwrap_or(""))
        .bind(quota_bytes.max(1_048_576))
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

        sqlx::query(
            "UPDATE mailboxes SET sync_status=CASE WHEN COALESCE(provider_account_id,'')='' THEN 'pending' ELSE 'retrying' END,
             sync_error='',updated_at=now() WHERE id=$1"
        )
        .bind(mailbox_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        if let Some(user_id)=mailbox_user {
            sqlx::query(
                "UPDATE users SET quota_bytes=$2,mail_sync_status=CASE WHEN COALESCE(mail_account_id,'')='' THEN 'pending' ELSE 'retrying' END,
                 mail_sync_error='',updated_at=now() WHERE id=$1 AND (active_mailbox_id=$3 OR primary_mailbox_id=$3)"
            )
            .bind(user_id).bind(quota_bytes.max(1_048_576)).bind(mailbox_id)
            .execute(&mut **tx).await.map_err(|e| ApiError::internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Queue a primary-password synchronization in the same transaction that
    /// updates the application password hash. The plaintext secret exists only
    /// long enough to be encrypted by PostgreSQL pgcrypto.
    pub async fn enqueue_credentials_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        _email: &str,
        _account_id: Option<&str>,
        password: &str,
    ) -> Result<(), ApiError> {
        let Some(target) = primary_mailbox_tx(tx, user_id).await? else {
            return Ok(());
        };
        let protected = Self::secret_payload(user_id, target.mailbox_id, &target.address, password);
        let dedupe = format!("mailbox.credentials:{}", target.mailbox_id);
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, secret_ciphertext,
                 status, max_attempts, dedupe_key)
             VALUES ($1,$2,$3,$4,$5,NULLIF($6,''),
                     pgp_sym_encrypt($7,$8,'cipher-algo=aes256, compress-algo=0'),
                     'pending',$9,$10)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id=EXCLUDED.user_id, organization_id=EXCLUDED.organization_id,
                           mailbox_id=EXCLUDED.mailbox_id, target_email=EXCLUDED.target_email,
                           account_id=COALESCE(EXCLUDED.account_id, provisioning_jobs.account_id),
                           secret_ciphertext=EXCLUDED.secret_ciphertext, status='pending', attempts=0,
                           max_attempts=EXCLUDED.max_attempts, next_attempt_at=now(), last_error='',
                           last_failure_transient=FALSE, completed_at=NULL, updated_at=now()",
        )
        .bind(user_id)
        .bind(target.organization_id)
        .bind(target.mailbox_id)
        .bind(OP_CREDENTIALS)
        .bind(&target.address)
        .bind(target.provider_account_id.as_deref().unwrap_or(""))
        .bind(protected)
        .bind(&self.inner.key)
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET sync_status='retrying', sync_error='', updated_at=now() WHERE id=$1")
            .bind(target.mailbox_id).execute(&mut **tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE users SET mail_sync_status='retrying', mail_sync_error='', updated_at=now() WHERE id=$1")
            .bind(user_id).execute(&mut **tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    /// Queue provider access enforcement for account suspension or reactivation.
    /// The worker reads the *current* user status when it executes, so a rapid
    /// suspend/reactivate sequence cannot apply stale provider state.
    pub async fn enqueue_access_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        _email: &str,
        _account_id: Option<&str>,
    ) -> Result<(), ApiError> {
        let Some(target) = primary_mailbox_tx(tx, user_id).await? else {
            return Ok(());
        };
        let dedupe = format!("mailbox.access:{}", target.mailbox_id);
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, status, max_attempts, dedupe_key)
             VALUES ($1,$2,$3,$4,$5,NULLIF($6,''),'pending',$7,$8)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id=EXCLUDED.user_id, organization_id=EXCLUDED.organization_id,
                           mailbox_id=EXCLUDED.mailbox_id, target_email=EXCLUDED.target_email,
                           account_id=COALESCE(EXCLUDED.account_id, provisioning_jobs.account_id),
                           status='pending', attempts=0, max_attempts=EXCLUDED.max_attempts,
                           next_attempt_at=now(), last_error='', last_failure_transient=FALSE,
                           completed_at=NULL, updated_at=now()",
        )
        .bind(user_id)
        .bind(target.organization_id)
        .bind(target.mailbox_id)
        .bind(OP_ACCESS)
        .bind(&target.address)
        .bind(target.provider_account_id.as_deref().unwrap_or(""))
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE mailboxes SET sync_status='retrying', sync_error='', updated_at=now() WHERE id=$1")
            .bind(target.mailbox_id).execute(&mut **tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        sqlx::query("UPDATE users SET mail_sync_status='retrying', mail_sync_error='', updated_at=now() WHERE id=$1")
            .bind(user_id).execute(&mut **tx).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

    pub async fn enqueue_delete_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        _email: &str,
        _account_id: Option<&str>,
    ) -> Result<(), ApiError> {
        let Some(target) = primary_mailbox_tx(tx, user_id).await? else {
            return Ok(());
        };
        let dedupe = format!("mailbox.delete:{}", target.mailbox_id);
        sqlx::query(
            "INSERT INTO provisioning_jobs
                (user_id, organization_id, mailbox_id, operation, target_email, account_id, status, max_attempts, dedupe_key)
             VALUES ($1,$2,$3,$4,$5,NULLIF($6,''),'pending',$7,$8)
             ON CONFLICT (dedupe_key) WHERE status IN ('pending','retry')
             DO UPDATE SET user_id=EXCLUDED.user_id, organization_id=EXCLUDED.organization_id,
                           mailbox_id=EXCLUDED.mailbox_id, target_email=EXCLUDED.target_email,
                           account_id=COALESCE(EXCLUDED.account_id, provisioning_jobs.account_id),
                           status='pending', attempts=0, max_attempts=EXCLUDED.max_attempts,
                           next_attempt_at=now(), last_error='', last_failure_transient=FALSE,
                           completed_at=NULL, updated_at=now()",
        )
        .bind(user_id)
        .bind(target.organization_id)
        .bind(target.mailbox_id)
        .bind(OP_DELETE)
        .bind(&target.address)
        .bind(target.provider_account_id.as_deref().unwrap_or(""))
        .bind(self.inner.max_attempts)
        .bind(dedupe)
        .execute(&mut **tx)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(())
    }

}

#[derive(Debug, sqlx::FromRow)]
struct JobRow {
    id: Uuid,
    user_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    mailbox_id: Option<Uuid>,
    operation: String,
    target_email: String,
    account_id: Option<String>,
    has_secret: bool,
    attempts: i32,
    max_attempts: i32,
    dedupe_key: String,
}

#[derive(Debug)]
struct JobFailure {
    message: String,
    transient: bool,
}

impl JobFailure {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }

    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }

    fn provider(error: StalwartError) -> Self {
        let transient = error.is_transient() || matches!(&error, StalwartError::Disabled);
        Self {
            message: error.to_string(),
            transient,
        }
    }
}

fn truncate(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

#[derive(Debug, sqlx::FromRow)]
struct JobMailbox {
    mailbox_id: Uuid,
    organization_id: Uuid,
    organization_status: String,
    subscription_status: String,
    address: String,
    local_part: String,
    provider_marker: String,
    provider_domain_id: Option<String>,
    domain_status: String,
    domain_is_system: bool,
    quota_bytes: i64,
    mailbox_status: String,
    provider_account_id: Option<String>,
    user_id: Option<Uuid>,
    user_status: Option<String>,
    membership_status: Option<String>,
}

async fn resolve_job_mailbox(state: &AppState, job: &JobRow) -> Result<JobMailbox, JobFailure> {
    let mailbox_id = job
        .mailbox_id
        .ok_or_else(|| JobFailure::permanent("Provisioning job is not bound to a hosted mailbox"))?;
    let mailbox = sqlx::query_as::<_, JobMailbox>(
        "SELECT m.id AS mailbox_id, m.organization_id, o.status AS organization_status,
                CASE
                  WHEN s.status IN ('active','trial') AND s.current_period_end IS NOT NULL AND s.current_period_end <= now() THEN 'past_due'
                  WHEN s.status='past_due' AND s.renewal_grace_end IS NOT NULL AND s.renewal_grace_end <= now() THEN 'suspended'
                  ELSE s.status
                END AS subscription_status,
                m.address::text AS address, m.local_part, m.provider_marker, d.provider_domain_id, d.status AS domain_status, d.is_system AS domain_is_system,
                m.quota_bytes, m.status AS mailbox_status, m.provider_account_id,
                m.user_id, u.status AS user_status, om.status AS membership_status
         FROM mailboxes m
         JOIN organizations o ON o.id=m.organization_id
         JOIN organization_subscriptions s ON s.organization_id=m.organization_id
         JOIN organization_domains d ON d.id=m.domain_id AND d.organization_id=m.organization_id
         LEFT JOIN users u ON u.id=m.user_id
         LEFT JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=m.user_id
         WHERE m.id=$1 AND m.deleted_at IS NULL",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| JobFailure::transient(e.to_string()))?
    .ok_or_else(|| JobFailure::permanent("Hosted mailbox no longer exists"))?;

    if job.organization_id.is_some_and(|id| id != mailbox.organization_id) {
        return Err(JobFailure::permanent("Provisioning organization binding changed"));
    }
    if !mailbox.address.eq_ignore_ascii_case(&job.target_email) {
        return Err(JobFailure::permanent("Provisioning mailbox address binding changed"));
    }
    if let (Some(job_user), Some(mailbox_user)) = (job.user_id, mailbox.user_id) {
        if job_user != mailbox_user {
            return Err(JobFailure::permanent("Provisioning mailbox ownership changed"));
        }
    }
    Ok(mailbox)
}

async fn mark_mailbox_ready(
    state: &AppState,
    mailbox: &JobMailbox,
    account_id: &str,
) -> Result<(), JobFailure> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    sqlx::query(
        "UPDATE mailboxes
         SET provider_account_id=$2, sync_status='ready', sync_error='',
             status=CASE WHEN status='suspended' THEN 'suspended' ELSE 'active' END,
             updated_at=now()
         WHERE id=$1",
    )
    .bind(mailbox.mailbox_id)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| JobFailure::transient(e.to_string()))?;
    if let Some(user_id) = mailbox.user_id {
        sqlx::query(
            "UPDATE users SET mail_account_id=$1, mail_sync_status='ready', mail_sync_error='',
             mail_synced_at=now(), updated_at=now()
             WHERE id=$2 AND primary_mailbox_id=$3",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(mailbox.mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    Ok(())
}

/// Claim and process this user's queued mailbox creation immediately. The job
/// remains durable if the provider is unavailable, but signup/admin-create can
/// still provision synchronously when Stalwart is healthy (important when a
/// verification message is sent to the newly-created mailbox).
pub async fn process_user_ensure_now(state: &AppState, user_id: Uuid) {
    let worker_id = format!("inline:{}", Uuid::new_v4());
    let claimed = sqlx::query_as::<_, JobRow>(
        "UPDATE provisioning_jobs AS job
         SET status = 'processing', attempts = job.attempts + 1,
             locked_at = now(), locked_by = $2, updated_at = now()
         WHERE job.id = (
             SELECT id FROM provisioning_jobs
             WHERE user_id = $1 AND operation = 'ensure_mailbox'
               AND status IN ('pending','retry') AND next_attempt_at <= now()
             ORDER BY created_at ASC
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING job.id, job.user_id, job.organization_id, job.mailbox_id, job.operation, job.target_email::text,
                   job.account_id,
                   (job.secret_ciphertext IS NOT NULL) AS has_secret,
                   job.attempts, job.max_attempts, job.dedupe_key",
    )
    .bind(user_id)
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await;

    match claimed {
        Ok(Some(job)) => process_claimed(state, &job).await,
        Ok(None) => {}
        Err(error) => tracing::warn!(%user_id, %error, "could not claim inline provisioning job"),
    }
}

/// Best-effort fast path for a queued credential synchronization. The queue
/// remains authoritative if the provider is unavailable.
pub async fn process_user_credentials_now(state: &AppState, user_id: Uuid) {
    let worker_id = format!("inline:{}", Uuid::new_v4());
    let claimed = sqlx::query_as::<_, JobRow>(
        "UPDATE provisioning_jobs AS job
         SET status = 'processing', attempts = job.attempts + 1,
             locked_at = now(), locked_by = $2, updated_at = now()
         WHERE job.id = (
             SELECT id FROM provisioning_jobs
             WHERE user_id = $1 AND operation = 'set_credentials'
               AND status IN ('pending','retry') AND next_attempt_at <= now()
             ORDER BY created_at ASC
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING job.id, job.user_id, job.organization_id, job.mailbox_id, job.operation, job.target_email::text,
                   job.account_id,
                   (job.secret_ciphertext IS NOT NULL) AS has_secret,
                   job.attempts, job.max_attempts, job.dedupe_key",
    )
    .bind(user_id)
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await;

    match claimed {
        Ok(Some(job)) => process_claimed(state, &job).await,
        Ok(None) => {}
        Err(error) => tracing::warn!(%user_id, %error, "could not claim inline credential job"),
    }
}

/// Best-effort fast path for account suspension/reactivation. The durable
/// `set_access` job remains authoritative if the provider is temporarily down.
pub async fn process_user_access_now(state: &AppState, user_id: Uuid) {
    let worker_id = format!("inline:{}", Uuid::new_v4());
    let claimed = sqlx::query_as::<_, JobRow>(
        "UPDATE provisioning_jobs AS job
         SET status = 'processing', attempts = job.attempts + 1,
             locked_at = now(), locked_by = $2, updated_at = now()
         WHERE job.id = (
             SELECT id FROM provisioning_jobs
             WHERE user_id = $1 AND operation = 'set_access'
               AND status IN ('pending','retry') AND next_attempt_at <= now()
             ORDER BY created_at ASC
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING job.id, job.user_id, job.organization_id, job.mailbox_id, job.operation, job.target_email::text,
                   job.account_id,
                   (job.secret_ciphertext IS NOT NULL) AS has_secret,
                   job.attempts, job.max_attempts, job.dedupe_key",
    )
    .bind(user_id)
    .bind(worker_id)
    .fetch_optional(&state.db)
    .await;

    match claimed {
        Ok(Some(job)) => process_claimed(state, &job).await,
        Ok(None) => {}
        Err(error) => tracing::warn!(%user_id, %error, "could not claim inline access job"),
    }
}

/// Start the durable queue worker. One process is enough, but the claim query
/// is safe when several Axum instances run against the same PostgreSQL DB.
pub fn spawn_worker(state: AppState) {
    if !state.stalwart.enabled() {
        tracing::info!("mail provider disabled; durable provisioning worker is paused");
        return;
    }
    tokio::spawn(async move {
        let worker_id = format!("{}:{}", std::process::id(), Uuid::new_v4());
        let mut tick = tokio::time::interval(state.provisioning.inner.poll_interval);
        let mut last_reconcile = Instant::now()
            .checked_sub(state.provisioning.inner.reconcile_interval)
            .unwrap_or_else(Instant::now);

        loop {
            tick.tick().await;

            if let Err(error) = recover_expired_leases(&state).await {
                tracing::warn!(%error, "provisioning lease recovery failed");
            }

            match claim_jobs(&state, &worker_id).await {
                Ok(jobs) => {
                    for job in jobs {
                        process_claimed(&state, &job).await;
                    }
                }
                Err(error) => tracing::warn!(%error, "provisioning job claim failed"),
            }

            if last_reconcile.elapsed() >= state.provisioning.inner.reconcile_interval {
                if let Err(error) = redrive_transient_dead(&state).await {
                    tracing::warn!(%error, "provisioning dead-letter redrive failed");
                }
                if let Err(error) = reconcile_users(&state).await {
                    tracing::warn!(%error, "mailbox reconciliation pass failed");
                }
                if let Err(error) = cleanup_jobs(&state.db).await {
                    tracing::warn!(%error, "provisioning job cleanup failed");
                }
                last_reconcile = Instant::now();
            }
        }
    });
}

async fn recover_expired_leases(state: &AppState) -> Result<(), sqlx::Error> {
    let lease_secs = state.provisioning.inner.lease_timeout.as_secs().min(i64::MAX as u64) as i64;
    let expired = sqlx::query_as::<_, JobRow>(
        "SELECT id, user_id, organization_id, mailbox_id, operation, target_email::text, account_id,
                (secret_ciphertext IS NOT NULL) AS has_secret,
                attempts, max_attempts, dedupe_key
         FROM provisioning_jobs
         WHERE status = 'processing'
           AND locked_at < now() - ($1 * interval '1 second')
         ORDER BY locked_at ASC",
    )
    .bind(lease_secs)
    .fetch_all(&state.db)
    .await?;

    let recovered = expired.len();
    for job in expired {
        // Reuse the normal failure path: it handles retry backoff, max-attempt
        // dead-lettering, and—critically—supersedes this stale lease when a
        // newer job with the same dedupe key was queued while it was running.
        fail_job(state, &job, JobFailure::transient("worker lease expired")).await?;
    }
    if recovered > 0 {
        tracing::warn!(recovered, "recovered expired provisioning leases");
    }
    Ok(())
}

async fn claim_jobs(state: &AppState, worker_id: &str) -> Result<Vec<JobRow>, sqlx::Error> {
    sqlx::query_as::<_, JobRow>(
        "UPDATE provisioning_jobs AS job
         SET status = 'processing', attempts = job.attempts + 1,
             locked_at = now(), locked_by = $1, updated_at = now()
         WHERE job.id IN (
             SELECT id FROM provisioning_jobs
             WHERE status IN ('pending','retry') AND next_attempt_at <= now()
               AND (operation <> 'ensure_mailbox' OR COALESCE((SELECT mailbox_provisioning_enabled FROM platform_controls WHERE singleton=TRUE), FALSE))
             ORDER BY next_attempt_at ASC, created_at ASC
             FOR UPDATE SKIP LOCKED
             LIMIT $2
         )
         RETURNING job.id, job.user_id, job.organization_id, job.mailbox_id, job.operation, job.target_email::text,
                   job.account_id,
                   (job.secret_ciphertext IS NOT NULL) AS has_secret,
                   job.attempts, job.max_attempts, job.dedupe_key",
    )
    .bind(worker_id)
    .bind(state.provisioning.inner.batch_size)
    .fetch_all(&state.db)
    .await
}

async fn process_claimed(state: &AppState, job: &JobRow) {
    let result = match job.operation.as_str() {
        OP_ENSURE => process_ensure(state, job).await,
        OP_QUOTA => process_quota(state, job).await,
        OP_CREDENTIALS => process_credentials(state, job).await,
        OP_ACCESS => process_access(state, job).await,
        OP_DELETE => process_delete(state, job).await,
        other => Err(JobFailure::permanent(format!("Unknown provisioning operation: {other}"))),
    };

    match result {
        Ok(()) => {
            if let Err(error) = complete_job(&state.db, job.id).await {
                tracing::warn!(job_id = %job.id, %error, "could not complete provisioning job");
            }
        }
        Err(failure) => {
            tracing::warn!(
                job_id = %job.id,
                operation = %job.operation,
                attempt = job.attempts,
                transient = failure.transient,
                error = %failure.message,
                "provisioning job failed"
            );
            if let Err(error) = fail_job(state, job, failure).await {
                tracing::warn!(job_id = %job.id, %error, "could not persist provisioning failure");
            }
        }
    }
}

async fn process_ensure(state: &AppState, job: &JobRow) -> Result<(), JobFailure> {
    if !state.stalwart.enabled() {
        return Err(JobFailure::transient("Mail provider is disabled"));
    }
    let user_id = job
        .user_id
        .ok_or_else(|| JobFailure::permanent("Provisioning user no longer exists"))?;
    let mailbox = resolve_job_mailbox(state, job).await?;
    if mailbox.mailbox_status == "deleting" {
        return Err(JobFailure::permanent("Hosted mailbox is being deleted"));
    }
    if mailbox.user_id != Some(user_id) {
        return Err(JobFailure::permanent("Hosted mailbox is no longer assigned to this user"));
    }
    if matches!(mailbox.mailbox_status.as_str(), "deleting" | "error") {
        return Err(JobFailure::permanent("Hosted mailbox is not provisionable in its current state"));
    }
    let password = state.provisioning.decrypt_secret(&state.db, job).await?;
    if mailbox.organization_status != "active" {
        return Err(JobFailure::transient("Hosted business is not active"));
    }
    if !matches!(mailbox.subscription_status.as_str(), "active" | "trial" | "past_due") {
        return Err(JobFailure::transient("Hosted business subscription does not allow mailbox provisioning"));
    }
    if mailbox.domain_status != "active" {
        return Err(JobFailure::transient("Hosted domain is not active"));
    }
    let account_id = if mailbox.domain_is_system {
        state.stalwart.ensure_mailbox_with_quota(&mailbox.address, &password, mailbox.quota_bytes.max(0) as u64)
            .await.map_err(JobFailure::provider)?
    } else {
        let provider_domain_id = mailbox.provider_domain_id.as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| JobFailure::transient("Hosted domain has not been provisioned"))?;
        if mailbox.provider_marker.trim().is_empty() {
            return Err(JobFailure::permanent("Hosted mailbox ownership marker is missing"));
        }
        state.stalwart.ensure_customer_mailbox_with_quota(
            provider_domain_id,
            &mailbox.local_part,
            &mailbox.provider_marker,
            &password,
            mailbox.quota_bytes.max(0) as u64,
        ).await.map_err(JobFailure::provider)?
    }.ok_or_else(|| JobFailure::transient("Mail provider did not return an account id"))?;
    let suspended = mailbox.organization_status != "active"
        || !matches!(mailbox.subscription_status.as_str(), "active" | "trial" | "past_due")
        || mailbox.domain_status != "active"
        || mailbox.mailbox_status == "suspended"
        || mailbox.user_status.as_deref() == Some("suspended")
        || (mailbox.user_id.is_some() && mailbox.membership_status.as_deref() != Some("active"));
    state
        .stalwart
        .set_account_suspended(&account_id, suspended)
        .await
        .map_err(JobFailure::provider)?;
    mark_mailbox_ready(state, &mailbox, &account_id).await
}

async fn refresh_provider_account_binding(
    state: &AppState,
    mailbox: &JobMailbox,
    account_id: &str,
) -> Result<(), JobFailure> {
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    sqlx::query(
        "UPDATE mailboxes SET provider_account_id=$2, updated_at=now()
         WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(mailbox.mailbox_id)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| JobFailure::transient(e.to_string()))?;
    if let Some(user_id) = mailbox.user_id {
        sqlx::query(
            "UPDATE users SET mail_account_id=$1, updated_at=now()
             WHERE id=$2 AND primary_mailbox_id=$3",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(mailbox.mailbox_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e| JobFailure::transient(e.to_string()))?;
    Ok(())
}

async fn resolve_owned_provider_account(
    state: &AppState,
    mailbox: &JobMailbox,
    job: &JobRow,
) -> Result<String, JobFailure> {
    let candidate = mailbox
        .provider_account_id
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| job.account_id.clone().filter(|value| !value.is_empty()));
    let provider_domain_id = mailbox.provider_domain_id.as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| JobFailure::transient("Hosted domain has not been provisioned"))?;
    let found = if mailbox.domain_is_system {
        state.stalwart.find_account_by_email(&mailbox.address).await.map_err(JobFailure::provider)?
    } else {
        state.stalwart.find_customer_account(provider_domain_id, &mailbox.local_part, &mailbox.provider_marker)
            .await.map_err(JobFailure::provider)?
    }.ok_or_else(|| JobFailure::transient("Mailbox has not been provisioned yet"))?;
    if candidate.as_deref().is_some_and(|stored| stored != found.as_str()) {
        tracing::warn!(
            mailbox_id = %mailbox.mailbox_id,
            address = %mailbox.address,
            stored_provider_account_id = ?candidate.as_deref(),
            resolved_provider_account_id = %found,
            "refreshing stale provider account id from verified ownership binding"
        );
        refresh_provider_account_binding(state, mailbox, &found).await?;
    }
    Ok(found)
}

async fn process_quota(state: &AppState, job: &JobRow) -> Result<(), JobFailure> {
    if !state.stalwart.enabled() {
        return Err(JobFailure::transient("Mail provider is disabled"));
    }
    let mailbox = resolve_job_mailbox(state, job).await?;
    if mailbox.mailbox_status == "deleting" {
        return Err(JobFailure::permanent("Hosted mailbox is being deleted"));
    }
    let account_id = resolve_owned_provider_account(state, &mailbox, job).await?;
    state
        .stalwart
        .set_account_quota(&account_id, mailbox.quota_bytes.max(0) as u64)
        .await
        .map_err(JobFailure::provider)?;
    mark_mailbox_ready(state, &mailbox, &account_id).await
}

async fn process_credentials(state: &AppState, job: &JobRow) -> Result<(), JobFailure> {
    if !state.stalwart.enabled() {
        return Err(JobFailure::transient("Mail provider is disabled"));
    }
    let user_id = job
        .user_id
        .ok_or_else(|| JobFailure::permanent("Provisioning user no longer exists"))?;
    let mailbox = resolve_job_mailbox(state, job).await?;
    if mailbox.mailbox_status == "deleting" {
        return Err(JobFailure::permanent("Hosted mailbox is being deleted"));
    }
    if mailbox.user_id != Some(user_id) {
        return Err(JobFailure::permanent("Hosted mailbox is no longer assigned to this user"));
    }
    let password = state.provisioning.decrypt_secret(&state.db, job).await?;
    let account = resolve_owned_provider_account(state, &mailbox, job).await?;
    state
        .stalwart
        .set_account_password(&account, &password)
        .await
        .map_err(JobFailure::provider)?;
    mark_mailbox_ready(state, &mailbox, &account).await
}

async fn process_access(state: &AppState, job: &JobRow) -> Result<(), JobFailure> {
    if !state.stalwart.enabled() {
        return Err(JobFailure::transient("Mail provider is disabled"));
    }
    let mailbox = resolve_job_mailbox(state, job).await?;
    if mailbox.mailbox_status == "deleting" {
        return Err(JobFailure::permanent("Hosted mailbox is being deleted"));
    }
    let account = resolve_owned_provider_account(state, &mailbox, job).await?;
    let suspended = mailbox.organization_status != "active"
        || !matches!(mailbox.subscription_status.as_str(), "active" | "trial" | "past_due")
        || mailbox.domain_status != "active"
        || mailbox.mailbox_status == "suspended"
        || mailbox.user_status.as_deref() == Some("suspended")
        || (mailbox.user_id.is_some() && mailbox.membership_status.as_deref() != Some("active"));
    state
        .stalwart
        .set_account_suspended(&account, suspended)
        .await
        .map_err(JobFailure::provider)?;
    mark_mailbox_ready(state, &mailbox, &account).await
}

async fn process_delete(state: &AppState, job: &JobRow) -> Result<(), JobFailure> {
    if !state.stalwart.enabled() { return Err(JobFailure::transient("Mail provider is disabled")); }
    let mailbox_id=job.mailbox_id.ok_or_else(|| JobFailure::permanent("Deletion job is not bound to a hosted mailbox"))?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=$1 AND deleted_at IS NULL)")
        .bind(mailbox_id).fetch_one(&state.db).await.map_err(|e|JobFailure::transient(e.to_string()))?;
    if !exists { return Ok(()); }
    let mailbox=resolve_job_mailbox(state,job).await?;
    if mailbox.mailbox_status!="deleting" { return Err(JobFailure::permanent("Hosted mailbox is no longer marked for deletion")); }
    if mailbox.domain_is_system { return Err(JobFailure::permanent("Automatic deletion of protected system-domain provider accounts is disabled on the shared mail server")); }
    let provider_domain_id=mailbox.provider_domain_id.as_deref().filter(|v|!v.trim().is_empty())
        .ok_or_else(||JobFailure::permanent("Hosted mailbox provider domain binding is missing"))?;
    let found=state.stalwart.find_customer_account(provider_domain_id,&mailbox.local_part,&mailbox.provider_marker).await.map_err(JobFailure::provider)?;
    if let Some(account)=found {
        let expected=mailbox.provider_account_id.clone().filter(|v|!v.is_empty()).or_else(||job.account_id.clone().filter(|v|!v.is_empty()));
        if expected.as_deref().is_some_and(|stored|stored!=account.as_str()) {
            tracing::warn!(
                mailbox_id = %mailbox.mailbox_id,
                address = %mailbox.address,
                stored_provider_account_id = ?expected.as_deref(),
                resolved_provider_account_id = %account,
                "refreshing stale provider account id before verified mailbox deletion"
            );
            refresh_provider_account_binding(state,&mailbox,&account).await?;
        }
        state.stalwart.destroy_customer_account(&account,provider_domain_id,&mailbox.local_part,&mailbox.provider_marker).await.map_err(JobFailure::provider)?;
    }
    let objects:Vec<(String,String)>=sqlx::query_as("SELECT storage_key,storage_backend FROM staged_attachments WHERE mailbox_id=$1")
        .bind(mailbox.mailbox_id).fetch_all(&state.db).await.map_err(|e|JobFailure::transient(e.to_string()))?;
    for (key,backend) in objects { state.object_store.delete(&key,&backend).await.map_err(JobFailure::transient)?; }
    let imports:Vec<(String,)>=sqlx::query_as("SELECT storage_key FROM mailbox_imports WHERE mailbox_id=$1")
        .bind(mailbox.mailbox_id).fetch_all(&state.db).await.map_err(|e|JobFailure::transient(e.to_string()))?;
    for (key,) in imports { state.object_store.delete_local(&key).await.map_err(JobFailure::transient)?; }
    state.object_store.delete_local_mailbox_dirs(mailbox.mailbox_id).await.map_err(JobFailure::transient)?;
    sqlx::query("UPDATE provisioning_jobs SET target_email='deleted-mailbox@invalid',account_id=NULL,secret_ciphertext=NULL,updated_at=now() WHERE mailbox_id=$1")
        .bind(mailbox.mailbox_id).execute(&state.db).await.map_err(|e|JobFailure::transient(e.to_string()))?;
    let deleted=sqlx::query("DELETE FROM mailboxes WHERE id=$1 AND status='deleting' AND deleted_at IS NULL")
        .bind(mailbox.mailbox_id).execute(&state.db).await.map_err(|e|JobFailure::transient(e.to_string()))?;
    if deleted.rows_affected()!=1 { return Err(JobFailure::transient("Mailbox deletion state changed before final database purge")); }
    Ok(())
}

async fn complete_job(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE provisioning_jobs
         SET status = 'succeeded', locked_at = NULL, locked_by = NULL,
             last_error = '', last_failure_transient = FALSE, secret_ciphertext = NULL,
             completed_at = now(), updated_at = now()
         WHERE id = $1 AND status = 'processing'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_superseded(pool: &PgPool, job: &JobRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE provisioning_jobs
         SET status = 'succeeded', locked_at = NULL, locked_by = NULL,
             last_error = 'Superseded by a newer queued operation',
             last_failure_transient = FALSE, secret_ciphertext = NULL,
             completed_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(job.id)
    .execute(pool)
    .await?;
    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

async fn fail_job(state: &AppState, job: &JobRow, failure: JobFailure) -> Result<(), sqlx::Error> {
    let dead = !failure.transient || job.attempts >= job.max_attempts;
    let message = truncate(&failure.message, MAX_ERROR_CHARS);
    if dead {
        sqlx::query(
            "UPDATE provisioning_jobs
             SET status = 'dead', locked_at = NULL, locked_by = NULL,
                 last_error = $2, last_failure_transient = $3,
                 secret_ciphertext = CASE WHEN $3 THEN secret_ciphertext ELSE NULL END,
                 completed_at = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(job.id)
        .bind(&message)
        .bind(failure.transient)
        .execute(&state.db)
        .await?;
    } else {
        // A newer active operation with the same semantic key makes retrying
        // this stale processing row both unnecessary and unsafe. Compare the
        // immutable creation tuple so two processing leases cannot supersede
        // each other symmetrically.
        let newer_active: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM provisioning_jobs candidate
                WHERE candidate.dedupe_key = $1
                  AND candidate.id <> $2
                  AND candidate.status IN ('pending','processing','retry')
                  AND (candidate.created_at, candidate.id) > (
                      SELECT current.created_at, current.id
                      FROM provisioning_jobs current WHERE current.id = $2
                  )
             )",
        )
        .bind(&job.dedupe_key)
        .bind(job.id)
        .fetch_one(&state.db)
        .await?;
        if newer_active {
            mark_superseded(&state.db, job).await?;
            return Ok(());
        }

        let exponent = (job.attempts.saturating_sub(1) as u32).min(8);
        let factor = 1u64 << exponent;
        let delay = state
            .provisioning
            .inner
            .retry_base
            .as_secs()
            .saturating_mul(factor)
            .clamp(1, 3600) as i64;
        let result = sqlx::query(
            "UPDATE provisioning_jobs
             SET status = 'retry', locked_at = NULL, locked_by = NULL,
                 last_error = $2, last_failure_transient = TRUE,
                 next_attempt_at = now() + ($3 * interval '1 second'), updated_at = now()
             WHERE id = $1",
        )
        .bind(job.id)
        .bind(&message)
        .bind(delay)
        .execute(&state.db)
        .await;
        match result {
            Ok(_) => {}
            // A concurrent enqueue can win between the EXISTS check and this
            // UPDATE. Treat that as supersession rather than losing the job
            // worker to a uniqueness error.
            Err(error) if is_unique_violation(&error) => {
                mark_superseded(&state.db, job).await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    let sync_state = if dead { "error" } else { "retrying" };
    if let Some(mailbox_id) = job.mailbox_id {
        sqlx::query(
            "UPDATE mailboxes SET sync_status=$2, sync_error=$3,
            status=CASE WHEN $2='error' AND $4=$5 AND status NOT IN ('suspended','deleting') THEN 'error' ELSE status END,
             updated_at=now() WHERE id=$1",
        )
        .bind(mailbox_id)
        .bind(sync_state)
        .bind(&message)
        .bind(&job.operation)
        .bind(OP_ENSURE)
        .execute(&state.db)
        .await?;
    }
    if let (Some(user_id), Some(mailbox_id)) = (job.user_id, job.mailbox_id) {
        sqlx::query(
            "UPDATE users SET mail_sync_status = $2, mail_sync_error = $3, updated_at = now()
             WHERE id = $1 AND primary_mailbox_id=$4",
        )
        .bind(user_id)
        .bind(sync_state)
        .bind(message)
        .bind(mailbox_id)
        .execute(&state.db)
        .await?;
    }
    Ok(())
}

/// Redrive transient dead letters after the provider itself is healthy again.
/// Encrypted ensure-mailbox credentials are retained only for this recovery
/// window; permanent failures have their secret erased immediately.
async fn redrive_transient_dead(state: &AppState) -> Result<(), sqlx::Error> {
    if !state.stalwart.enabled() || state.stalwart.healthcheck().await.is_err() {
        return Ok(());
    }
    let redriven = sqlx::query(
        "UPDATE provisioning_jobs AS dead
         SET status = 'retry', attempts = 0, next_attempt_at = now(),
             locked_at = NULL, locked_by = NULL, completed_at = NULL,
             updated_at = now()
         WHERE dead.status = 'dead' AND dead.last_failure_transient = TRUE
           AND dead.completed_at > now() - interval '7 days'
           AND NOT EXISTS (
               SELECT 1 FROM provisioning_jobs queued
               WHERE queued.dedupe_key = dead.dedupe_key
                 AND queued.status IN ('pending','retry')
           )
           AND dead.id = (
               SELECT candidate.id FROM provisioning_jobs candidate
               WHERE candidate.dedupe_key = dead.dedupe_key
                 AND candidate.status = 'dead'
                 AND candidate.last_failure_transient = TRUE
                 AND candidate.completed_at > now() - interval '7 days'
               ORDER BY candidate.completed_at DESC, candidate.created_at DESC, candidate.id DESC
               LIMIT 1
           )",
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    if redriven > 0 {
        tracing::info!(redriven, "redriving transient provisioning dead letters");
    }
    Ok(())
}

/// Periodically verifies provider linkage and quota for existing rows. This
/// catches manual provider edits and pre-upgrade accounts that have a valid
/// Stalwart mailbox but no cached `mail_account_id`.
async fn reconcile_users(state: &AppState) -> Result<(), sqlx::Error> {
    if !state.stalwart.enabled() {
        return Ok(());
    }
    let rows: Vec<(Uuid, Uuid, Uuid, String, String, String, Option<String>, bool, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT COALESCE(m.user_id,o.created_by), m.organization_id, m.id, m.address::text, m.local_part, m.provider_marker, d.provider_domain_id, d.is_system,
                COALESCE(m.quota_override_bytes, s.mailbox_quota_override_bytes, pv.mailbox_bytes, p.mailbox_bytes)::bigint AS effective_quota,
                m.quota_bytes AS materialized_quota,
                m.provider_account_id
         FROM mailboxes m
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_subscriptions s ON s.organization_id=m.organization_id
         JOIN plans p ON p.code=s.plan_code
         LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
         JOIN organization_domains d ON d.id=m.domain_id AND d.status='active'
         WHERE m.deleted_at IS NULL AND m.status <> 'deleting'
           AND COALESCE(m.user_id,o.created_by) IS NOT NULL
           AND (m.provider_reconciled_at IS NULL OR m.provider_reconciled_at < now() - interval '15 minutes')
         ORDER BY m.provider_reconciled_at ASC NULLS FIRST, m.created_at ASC
         LIMIT 20",
    )
    .fetch_all(&state.db)
    .await?;

    for (user_id, organization_id, mailbox_id, email, local_part, provider_marker, provider_domain_id, domain_is_system, quota_bytes, materialized_quota, stored_account) in rows {
        if quota_bytes != materialized_quota {
            sqlx::query("UPDATE mailboxes SET quota_bytes = $2, updated_at = now() WHERE id = $1")
                .bind(mailbox_id)
                .bind(quota_bytes)
                .execute(&state.db)
                .await?;
        }
        let account = if let Some(account) = stored_account.filter(|value| !value.is_empty()) {
            Some(account)
        } else {
            if domain_is_system {
                match state.stalwart.find_account_by_email(&email).await {
                    Ok(found) => found,
                    Err(error) => {
                        let message = truncate(&error.to_string(), MAX_ERROR_CHARS);
                        let _ = sqlx::query("UPDATE users SET mail_sync_status='retrying',mail_sync_error=$2,mail_synced_at=now() WHERE id=$1")
                            .bind(user_id).bind(message).execute(&state.db).await;
                        mark_mailbox_reconciled(&state.db, mailbox_id).await?;
                        continue;
                    }
                }
            } else { match provider_domain_id.as_deref() {
                Some(domain_id) if !provider_marker.trim().is_empty() => {
                    match state.stalwart.find_customer_account(domain_id, &local_part, &provider_marker).await {
                        Ok(found) => found,
                        Err(error) => {
                            let message = truncate(&error.to_string(), MAX_ERROR_CHARS);
                            let _ = sqlx::query(
                                "UPDATE users SET mail_sync_status = 'retrying', mail_sync_error = $2,
                                                  mail_synced_at = now() WHERE id = $1",
                            )
                            .bind(user_id)
                            .bind(message)
                            .execute(&state.db)
                            .await;
                            mark_mailbox_reconciled(&state.db, mailbox_id).await?;
                            continue;
                        }
                    }
                }
                _ => {
                    let message = "Hosted mailbox provider binding is incomplete";
                    let _ = sqlx::query(
                        "UPDATE users SET mail_sync_status = 'retrying', mail_sync_error = $2,
                                          mail_synced_at = now() WHERE id = $1",
                    )
                    .bind(user_id)
                    .bind(message)
                    .execute(&state.db)
                    .await;
                    mark_mailbox_reconciled(&state.db, mailbox_id).await?;
                    continue;
                }
            } }
        };

        let Some(account) = account else {
            let pending: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                    SELECT 1 FROM provisioning_jobs
                    WHERE user_id = $1 AND operation = 'ensure_mailbox'
                      AND status IN ('pending','processing','retry')
                 )",
            )
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
            let (status, error) = if pending {
                ("pending", "")
            } else {
                (
                    "error",
                    "Mailbox is missing and no recoverable provisioning credential is queued",
                )
            };
            sqlx::query(
                "UPDATE users SET mail_sync_status = $2, mail_sync_error = $3,
                                  mail_synced_at = now() WHERE id = $1",
            )
            .bind(user_id)
            .bind(status)
            .bind(error)
            .execute(&state.db)
            .await?;
            mark_mailbox_reconciled(&state.db, mailbox_id).await?;
            continue;
        };

        match state.stalwart.account_quota(&account).await {
            Ok(Some((_used, provider_quota))) => {
                sqlx::query(
                    "UPDATE users SET mail_account_id = $1, mail_sync_status = 'ready',
                                      mail_sync_error = '', mail_synced_at = now()
                     WHERE id = $2",
                )
                .bind(&account)
                .bind(user_id)
                .execute(&state.db)
                .await?;
                if provider_quota != quota_bytes.max(0) as u64 {
                    if let Err(error) = state
                        .provisioning
                        .enqueue_quota(&state.db, user_id, &email, Some(&account), quota_bytes)
                        .await
                    {
                        tracing::warn!(%user_id, %error, "could not queue quota reconciliation");
                    }
                }
            }
            Ok(None) => {
                sqlx::query(
                    "UPDATE users SET mail_account_id = NULL, mail_sync_status = 'error',
                                      mail_sync_error = 'Stalwart mailbox id no longer exists',
                                      mail_synced_at = now() WHERE id = $1",
                )
                .bind(user_id)
                .execute(&state.db)
                .await?;
            }
            Err(error) => {
                let message = truncate(&error.to_string(), MAX_ERROR_CHARS);
                sqlx::query(
                    "UPDATE users SET mail_sync_status = 'retrying', mail_sync_error = $2,
                                      mail_synced_at = now() WHERE id = $1",
                )
                .bind(user_id)
                .bind(message)
                .execute(&state.db)
                .await?;
            }
        }

        // Provider reconciliation also reasserts access authority. This closes
        // drift caused by manual provider edits or a missed lifecycle job.
        sqlx::query(
            "INSERT INTO provisioning_jobs
              (user_id,organization_id,mailbox_id,operation,target_email,account_id,status,max_attempts,dedupe_key)
             VALUES($1,$2,$3,'set_access',$4,$5,'pending',8,'mailbox.access:'||$3::text)
             ON CONFLICT(dedupe_key) WHERE status IN('pending','retry') DO UPDATE SET
               user_id=EXCLUDED.user_id,organization_id=EXCLUDED.organization_id,target_email=EXCLUDED.target_email,
               account_id=COALESCE(EXCLUDED.account_id,provisioning_jobs.account_id),status='pending',attempts=0,
               next_attempt_at=now(),last_error='',last_failure_transient=FALSE,completed_at=NULL,updated_at=now()",
        )
        .bind(user_id)
        .bind(organization_id)
        .bind(mailbox_id)
        .bind(&email)
        .bind(&account)
        .execute(&state.db)
        .await?;
        mark_mailbox_reconciled(&state.db, mailbox_id).await?;
    }
    Ok(())
}

async fn mark_mailbox_reconciled(pool: &PgPool, mailbox_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mailboxes SET provider_reconciled_at = now() WHERE id = $1")
        .bind(mailbox_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn cleanup_jobs(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Do not retain even encrypted user credentials indefinitely. Seven days
    // is enough for automatic outage recovery; the diagnostic job row remains
    // for 30 days without the secret.
    sqlx::query(
        "UPDATE provisioning_jobs SET secret_ciphertext = NULL, updated_at = now()
         WHERE status = 'dead' AND completed_at < now() - interval '7 days'
           AND secret_ciphertext IS NOT NULL",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "DELETE FROM provisioning_jobs
         WHERE (status = 'succeeded' AND completed_at < now() - interval '7 days')
            OR (status = 'dead' AND completed_at < now() - interval '30 days')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_secret_payload_binds_user_and_email() {
        let user_id = Uuid::new_v4();
        let mailbox_id = Uuid::new_v4();
        let payload = ProvisioningService::secret_payload(user_id, mailbox_id, "Alice@Example.com", "secret");
        let mut parts = payload.splitn(4, '\n');
        let user_id_text = user_id.to_string();
        assert_eq!(parts.next(), Some(user_id_text.as_str()));
        let mailbox_id_text = mailbox_id.to_string();
        assert_eq!(parts.next(), Some(mailbox_id_text.as_str()));
        assert_eq!(parts.next(), Some("alice@example.com"));
        assert_eq!(parts.next(), Some("secret"));
    }

    #[test]
    fn service_rejects_invalid_worker_configuration() {
        assert!(ProvisioningService::new(
            "test-provisioning-key-0123456789abcdef".into(),
            Duration::ZERO,
            Duration::from_secs(60),
            Duration::from_secs(1),
            Duration::from_secs(60),
            3,
            5,
        )
        .is_err());
    }
}
