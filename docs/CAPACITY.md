# CS Mail production capacity and storage layout

This document describes the supported production storage split for a small CS Mail control-plane VPS. It is intentionally conservative: customer mailbox quotas are not the same thing as PostgreSQL disk capacity.

## Storage planes

CS Mail has three independent storage planes:

1. **PostgreSQL (local VPS)** — accounts, businesses, subscriptions, mailbox metadata, queues, drafts, audit/security records, import progress, notification state and indexes.
2. **CS Mail object storage** — staged/application attachment objects. In production set `CS_MAIL_OBJECT_STORAGE_BACKEND=r2` so durable attachment objects live in a private Cloudflare R2 bucket. `/srv/attachments` remains a bounded private spool and legacy-local-object location.
3. **Stalwart mailbox storage** — the actual message/blob store for hosted mailboxes. The shared Stalwart server owns this data. Configuring CS Mail's R2 bucket does **not** move Stalwart messages. To sell multi-gigabyte mailbox quotas from a small VPS, configure Stalwart's Blob Store to a separate S3-compatible R2 bucket.

Use separate buckets/credentials for CS Mail application objects and Stalwart message blobs. This limits blast radius and makes backup/retention policy easier to reason about.

## 20 GiB PostgreSQL operating envelope

`CS_MAIL_DB_CAPACITY_BYTES=21474836480` declares a 20 GiB PostgreSQL planning capacity. Do not plan to consume all 20 GiB. PostgreSQL needs free space for VACUUM, indexes, migrations, WAL/temp work and recovery operations.

- **Normal operating target:** below 70% = below ~14 GiB.
- **Warning:** 70% to 85% = ~14–17 GiB.
- **Critical:** 85% or more = ~17 GiB+; expand/migrate before normal growth continues.

`sh manage capacity` reports current database size, headroom, tenant counts, the largest relations, object-storage mode, local spool size and host filesystem use. `/api/metrics` also exports `cs_mail_database_bytes`, `cs_mail_database_capacity_ratio` and user/mailbox/business counts for Prometheus.

## Planning mailbox counts

The amount of PostgreSQL metadata per mailbox depends mostly on sending activity, drafts, audit volume, import history and administrative activity. It does **not** scale linearly with the gigabytes of messages stored in Stalwart/R2.

For a 14 GiB normal-operation budget, use these planning bands until production telemetry gives a measured number for your traffic:

| Workload profile | Approx. primary-DB footprint per active mailbox | 14 GiB planning capacity |
| --- | ---: | ---: |
| Light | 5 MiB | ~2,800 mailboxes |
| Typical business | 15 MiB | ~950 mailboxes |
| Heavy sender/import/admin | 50 MiB | ~280 mailboxes |

These are engineering planning assumptions, **not benchmark results**. CPU, RAM, SQL connection pressure, Stalwart indexing/JMAP/IMAP concurrency and outbound-delivery limits may become the bottleneck before disk does. For an initial single-VPS launch, keep an operational soft cap around **300–500 active mailboxes** until `sh manage capacity`, Prometheus, API latency and provider telemetry demonstrate safe headroom. Scale horizontally/vertically from measurements rather than raising the cap only because R2 has space.

A "user" is a platform identity and may or may not own a mailbox. Capacity should therefore be planned by **active mailboxes and workload**, not just login count. Thousands of low-activity platform identities can consume less capacity than a few hundred busy mailboxes.

## Mailbox quota versus local VPS disk

Current catalog quotas are much larger than a 20 GB local disk:

- CS Mail Start: 5 GiB per mailbox.
- CS Mail Grow: 3 included mailboxes / 30 GiB base storage pool.
- CS Mail Scale: 5 included mailboxes / 80 GiB base storage pool.

If Stalwart keeps message blobs on the same 20 GB local disk, Grow and Scale cannot be honestly provisioned at their advertised capacity, and even a handful of full Start mailboxes can exhaust the server after reserving space for the OS, PostgreSQL, indexes, logs and spool. This topology is suitable only for testing or very small quotas.

For production, keep the local VPS as the control/index plane and put Stalwart's Blob Store on R2/S3-compatible storage. R2 object storage does not make compute unlimited: Stalwart still needs enough local resources for its indexes/datastore, queues and active protocol workload.

## Retention controls in v9

To prevent metadata tables from growing without bound, the API maintenance worker now prunes:

- daily sending counters after 90 days;
- hourly sending counters after 30 days;
- dismissed notifications after 90 days and read notifications after 365 days;
- completed MBOX import ledgers after 30 days;
- failed/cancelled MBOX import ledgers after 90 days;
- existing send-request/scheduled-send/session/token cleanup remains in place.

Security/audit history is deliberately not silently deleted by this housekeeping. Monitor the audit table in `sh manage capacity`; if it becomes material, introduce an explicit compliance retention/export/partitioning policy rather than deleting evidence implicitly.

## R2 transfer efficiency

Set `CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS=4` initially. CS Mail bounds concurrent application-object transfers with a semaphore so several large attachments cannot independently consume unbounded API memory. Raise this only after observing API memory and network throughput; accepted range is 1–16.

The current attachment API still buffers an individual object in memory on download, so `max_attachment_bytes` and the R2 transfer limit remain important safeguards. Scale the API memory limit before increasing both attachment size and concurrent-transfer count.

## Stalwart + R2 production topology

Configure the shared Stalwart instance separately from CS Mail:

- Create a private bucket such as `cs-mail-stalwart-production`.
- Create a dedicated R2 Object Read & Write credential scoped only to that bucket.
- In Stalwart Webadmin, configure an **S3-compatible Blob Store** for message blobs.
- Use the R2 S3 endpoint `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`, region `auto`, the bucket name and the dedicated access/secret keys.
- Use a separate prefix such as `stalwart/mail` if desired.
- Keep TLS certificate validation enabled and start with write verification enabled; optimize only after measured production testing.
- Preserve a separate backup/recovery policy. Object storage is durable storage, not by itself protection against an authorized deletion or application bug.

This Stalwart configuration is not part of `/opt/cs-mail/.env.production` because Stalwart is a separately managed shared provider.

## Routine operation

Run:

```bash
sh manage capacity
```

at least weekly during early launch and before increasing plan/user limits. Investigate the largest PostgreSQL relations when utilization grows faster than active-mailbox count. Treat the capacity thresholds as operational guardrails, not as a promise that a 20 GiB database can safely reach 20 GiB.
