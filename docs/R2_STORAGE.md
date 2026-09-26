# Cloudflare R2 storage

CS Mail production should use **two separate storage responsibilities**:

1. a private R2 bucket for CS Mail application attachment objects; and
2. a separate private R2 bucket configured directly in Stalwart for hosted-mail message blobs.

The first is controlled by CS Mail environment variables. The second belongs to the separately managed shared Stalwart provider.

## CS Mail application-object bucket

Create a private R2 bucket, recommended name `cs-mail-production`. Create an R2 S3 API credential with **Object Read & Write** scoped only to that bucket. PostgreSQL remains the metadata/ownership authority. MBOX imports continue to use `/srv/attachments` as a private processing spool.

Set `/opt/cs-mail/.env.production`:

```env
CS_MAIL_ATTACHMENT_STORE_DIR=/srv/attachments
CS_MAIL_OBJECT_STORAGE_BACKEND=r2
CS_MAIL_R2_ACCOUNT_ID=<cloudflare-account-id>
CS_MAIL_R2_BUCKET=cs-mail-production
CS_MAIL_R2_ACCESS_KEY_ID=<r2-access-key-id>
CS_MAIL_R2_SECRET_ACCESS_KEY=<r2-secret-access-key>
CS_MAIL_R2_ENDPOINT=
CS_MAIL_R2_REGION=auto
CS_MAIL_R2_PREFIX=cs-mail/attachments
CS_MAIL_R2_REQUEST_TIMEOUT_SECS=30
CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS=4
```

Leave `CS_MAIL_R2_ENDPOINT` blank for the normal account endpoint (`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`). If the bucket uses a jurisdictional endpoint, put Cloudflare's exact HTTPS endpoint there. Keep public bucket access disabled.

`CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS` is intentionally bounded to 1–16. Start at 4 on a small API container. It caps simultaneous R2 uploads/downloads so large attachments cannot create unbounded memory pressure.

After editing the env file run:

```bash
sh manage preflight
sh manage deploy
sh manage capacity
```

Startup performs an authenticated bucket health check and fails closed if R2 is selected but unavailable. Migration 0050 records `storage_backend` per attachment, so existing local objects remain readable/deletable while new uploads use R2.

## Stalwart message-blob bucket

CS Mail's R2 variables do **not** move the actual mailbox message bodies. The shared Stalwart provider owns those blobs. To offer the current 5/10/15 GiB-per-mailbox plan quotas from a small VPS, configure Stalwart's **S3-compatible Blob Store** to a separate private R2 bucket, for example `cs-mail-stalwart-production`.

Use a separate R2 credential scoped only to that bucket. In Stalwart's storage configuration use the R2 endpoint `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`, region `auto`, the bucket and its dedicated access/secret keys. Keep TLS validation enabled. Start conservatively with write verification enabled and normal retry/timeout values, then tune only from observed production behavior.

Do not reuse the CS Mail application bucket unless you have a deliberate operational reason. Separate buckets make permissions, incident response, retention and recovery scope clearer.

R2 is the durable primary blob store in this topology, not an automatic backup. Maintain independent recovery evidence for both CS Mail data and Stalwart data before public launch.
