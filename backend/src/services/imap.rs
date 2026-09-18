//! Mail read/write path (WS2). Despite the historical filename, delivery is
//! NOT IMAP: it was redesigned to a single pinned admin credential calling the
//! Stalwart JMAP API (RFC 8620) on behalf of each user. Every method here runs
//! against the admin session with the target user's Stalwart account id in
//! `accountId` (admin impersonation) — we never store per-user mail secrets.
//!
//! Responsibilities: mailbox tree w/ counters, paginated thread previews,
//! full thread + bodies, read/starred flags, move, delete, text search, and
//! attachment blobs.

use serde_json::{json, Value};

use crate::services::provisioning::MailBridge;

/// Properties needed to render a list row / preview.
const LIST_PROPS: &[&str] = &[
    "id",
    "threadId",
    "mailboxIds",
    "keywords",
    "size",
    "date",
    "receivedAt",
    "from",
    "to",
    "cc",
    "bcc",
    "replyTo",
    "subject",
    "preview",
    "hasAttachment",
];

/// Properties for a full thread read, including body parts and the header
/// fields that drive quote-aware conversation threading.
const FULL_PROPS_FETCH: &[&str] = &[
    "id",
    "blobId",
    "threadId",
    "mailboxIds",
    "keywords",
    "size",
    "date",
    "receivedAt",
    "from",
    "to",
    "cc",
    "bcc",
    "replyTo",
    "subject",
    "preview",
    "hasAttachment",
    "attachments",
    "textBody",
    "htmlBody",
    "bodyValues",
    "header:Message-ID",
    "header:References",
    "header:In-Reply-To",
    // Deliverability/abuse signals (WS2.5): the RFC 7489 Authentication-Results
    // chain plus the spam-filter banners the relay stamps on delivery.
    "header:Authentication-Results",
    "header:Received-SPF",
    "header:X-Spam-Status",
    "header:X-Spam-Flag",
    "header:X-Spam-Score",
];

/// Compact mailbox roster with unread counts (sidebar).
pub async fn mailboxes(bridge: &MailBridge, account: &str) -> Result<Value, String> {
    let result = bridge
        .jmap_mail("Mailbox/get", json!({ "accountId": account }))
        .await?;
    let list = result
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(list.len());
    for mb in list {
        out.push(json!({
            "id": mb["id"],
            "name": mb["name"],
            "parent_id": mb.get("parentId"),
            "role": mb.get("role"),
            "sort_order": mb.get("sortOrder"),
            "subscribed": mb.get("isSubscribed"),
            "total": mb.get("totalEmails").and_then(Value::as_u64).unwrap_or(0),
            "unread": mb.get("unreadEmails").and_then(Value::as_u64).unwrap_or(0),
        }));
    }
    Ok(Value::Array(out))
}

/// One page of thread previews from a mailbox, newest first. Returns
/// `has_more` so the UI can paginate; `anchor` is the last email id seen
/// (older-than pagination).
pub async fn thread_previews(
    bridge: &MailBridge,
    account: &str,
    in_mailbox: &str,
    limit: usize,
    anchor: Option<&str>,
) -> Result<Value, String> {
    let mut args = json!({
        "accountId": account,
        "filter": { "inMailbox": in_mailbox },
        "sort": [{ "property": "receivedAt", "isAscending": false }],
        "collapseThreads": true,
        "limit": (limit as i64) + 1
    });
    if let Some(a) = anchor {
        args["anchor"] = json!(a);
        args["anchorOffset"] = json!(1);
    } else {
        args["position"] = json!(0);
    }

    let result = bridge.jmap_mail("Email/query", args).await?;
    let ids: Vec<Value> = result
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_more = ids.len() > limit;
    let ids: Vec<Value> = ids.into_iter().take(limit).collect();

    let emails = emails_by_id(bridge, account, &ids, LIST_PROPS, false).await?;
    Ok(json!({ "emails": to_rows(emails), "has_more": has_more }))
}

/// Full thread (conversation) with every message's bodies and headers.
pub async fn thread(bridge: &MailBridge, account: &str, thread_id: &str) -> Result<Value, String> {
    let t = bridge
        .jmap_mail(
            "Thread/get",
            json!({ "accountId": account, "ids": [thread_id] }),
        )
        .await?;
    let thread = t
        .get("list")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .ok_or_else(|| format!("thread {thread_id} not found"))?;
    let ids: Vec<Value> = thread
        .get("emailIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut emails = emails_by_id(bridge, account, &ids, FULL_PROPS_FETCH, true).await?;
    let junk_mailbox = junk_mailbox_id(bridge, account).await?;
    for e in &mut emails {
        let seen = has_keyword(e, "$seen");
        let starred = has_keyword(e, "$flagged");
        if let Some(obj) = e.as_object_mut() {
            obj.insert("body_html".into(), json!(body_for(obj, "htmlBody")));
            obj.insert("body_text".into(), json!(body_for(obj, "textBody")));
            obj.insert("seen".into(), json!(seen));
            obj.insert("starred".into(), json!(starred));
            obj.insert(
                "security".into(),
                security_verdicts(obj, junk_mailbox.as_deref()),
            );
        }
    }
    Ok(json!({
        "thread_id": thread_id,
        "count": emails.len(),
        "emails": emails
    }))
}

/// Batch the threads a text search matches, newest first (same shape as
/// `thread_previews`). Any property but `text` can be added later for
/// advanced search (WS4.x).
pub async fn search(
    bridge: &MailBridge,
    account: &str,
    query: &str,
    limit: usize,
) -> Result<Value, String> {
    let args = json!({
        "accountId": account,
        "filter": { "text": query },
        "sort": [{ "property": "receivedAt", "isAscending": false }],
        "collapseThreads": true,
        "position": 0,
        "limit": (limit as i64) + 1
    });
    let result = bridge.jmap_mail("Email/query", args).await?;
    let ids: Vec<Value> = result
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_more = ids.len() > limit;
    let ids: Vec<Value> = ids.into_iter().take(limit).collect();

    let emails = emails_by_id(bridge, account, &ids, LIST_PROPS, false).await?;
    Ok(json!({ "emails": to_rows(emails), "has_more": has_more }))
}

/// Mark a set of emails read/unread (`$seen`) and/or starred (`$flagged`).
/// Stalwart 0.16's `Email/set` expects a full `keywords` map, not the RFC 8621
/// `$is`/`$not` patch, so we merge with the current keywords first.
pub async fn set_flags(
    bridge: &MailBridge,
    account: &str,
    email_ids: &[String],
    read: Option<bool>,
    starred: Option<bool>,
) -> Result<(), String> {
    if email_ids.is_empty() {
        return Ok(());
    }
    let mut update = serde_json::Map::new();
    for (id, current) in current_map(bridge, account, email_ids, "keywords").await? {
        let mut keywords = current
            .as_object()
            .cloned()
            .unwrap_or_else(serde_json::Map::new);
        if let Some(r) = read {
            keywords.insert("$seen".into(), json!(r));
        }
        if let Some(s) = starred {
            keywords.insert("$flagged".into(), json!(s));
        }
        update.insert(id.clone(), json!({ "keywords": keywords }));
    }
    if update.is_empty() {
        return Ok(());
    }
    let result = bridge
        .jmap_mail(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Move emails into `to`; when the UI knows the current mailbox it passes
/// `from` so this is a true move. Full replacement map again (Stalwart 0.16
/// rejects/ignores the `$add`/`$remove` patch).
pub async fn move_emails(
    bridge: &MailBridge,
    account: &str,
    email_ids: &[String],
    from: Option<&str>,
    to: &str,
) -> Result<(), String> {
    if email_ids.is_empty() {
        return Ok(());
    }
    let mut update = serde_json::Map::new();
    for (id, current) in current_map(bridge, account, email_ids, "mailboxIds").await? {
        let mut mailboxes = current
            .as_object()
            .cloned()
            .unwrap_or_else(serde_json::Map::new);
        if let Some(f) = from {
            mailboxes.remove(f);
        }
        mailboxes.insert(to.into(), json!(true));
        update.insert(id.clone(), json!({ "mailboxIds": mailboxes }));
    }
    if update.is_empty() {
        return Ok(());
    }
    let result = bridge
        .jmap_mail(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Id of the `Sent` folder for an account (from the mailbox roster), so the
/// send path can file the sender's self-copy exactly where webmail shows it.
pub async fn sent_mailbox_id(bridge: &MailBridge, account: &str) -> Result<Option<String>, String> {
    let list = mailboxes(bridge, account).await?;
    for mb in list.as_array().into_iter().flatten() {
        if mb.get("role").and_then(Value::as_str) == Some("sent") {
            return Ok(mb.get("id").and_then(Value::as_str).map(String::from));
        }
    }
    Ok(None)
}

/// Id of the `Junk` folder, used to class a message as spam even when the
/// relay stamps no explicit spam header (WS2.5).
pub async fn junk_mailbox_id(bridge: &MailBridge, account: &str) -> Result<Option<String>, String> {
    let list = mailboxes(bridge, account).await?;
    for mb in list.as_array().into_iter().flatten() {
        if mb.get("role").and_then(Value::as_str) == Some("junk") {
            return Ok(mb.get("id").and_then(Value::as_str).map(String::from));
        }
    }
    Ok(None)
}

/// Id of the `Inbox` folder (realtime new-mail poller, WS2.6).
pub async fn inbox_mailbox_id(
    bridge: &MailBridge,
    account: &str,
) -> Result<Option<String>, String> {
    let list = mailboxes(bridge, account).await?;
    for mb in list.as_array().into_iter().flatten() {
        if mb.get("role").and_then(Value::as_str) == Some("inbox") {
            return Ok(mb.get("id").and_then(Value::as_str).map(String::from));
        }
    }
    Ok(None)
}

/// Inbox messages received after `since` (RFC 3339), newest first, as preview
/// rows. `since = None` means "give me the newest mail" — the poller uses that
/// on first sight to establish a watermark without replaying history.
pub async fn recent_inbox(
    bridge: &MailBridge,
    account: &str,
    since: Option<&str>,
    limit: usize,
) -> Result<Value, String> {
    let Some(inbox) = inbox_mailbox_id(bridge, account).await? else {
        return Ok(Value::Array(Vec::new()));
    };
    let mut filter = json!({ "inMailbox": inbox });
    if let Some(s) = since {
        filter["after"] = json!(s);
    }
    let result = bridge
        .jmap_mail(
            "Email/query",
            json!({
                "accountId": account,
                "filter": filter,
                "sort": [{ "property": "receivedAt", "isAscending": false }],
                "collapseThreads": false,
                "position": 0,
                "limit": limit as i64
            }),
        )
        .await?;
    let ids: Vec<Value> = result
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let emails = emails_by_id(bridge, account, &ids, LIST_PROPS, false).await?;
    Ok(Value::Array(to_rows(emails)))
}

/// Disk usage for one account from Stalwart's management namespace, as
/// `(used_bytes, quota_bytes)`. Returns `None` if the account has vanished.
pub async fn account_quota(
    bridge: &MailBridge,
    account: &str,
) -> Result<Option<(u64, u64)>, String> {
    if !bridge.enabled() {
        return Ok(None);
    }
    let result = bridge
        .jmap("x:Account/get", json!({ "ids": [account] }))
        .await?;
    let Some(item) = result
        .get("list")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    else {
        return Ok(None);
    };
    let used = item
        .get("usedDiskQuota")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = item
        .get("quotas")
        .and_then(|q| q.get("maxDiskQuota"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(Some((used, total)))
}

/// Batch disk usage for many accounts in one `x:Account/get` call, keyed by
/// Stalwart account id. The admin surface lists N users for one management
/// round-trip instead of N.
pub async fn account_quotas(
    bridge: &MailBridge,
    accounts: &[String],
) -> Result<std::collections::HashMap<String, (u64, u64)>, String> {
    let mut out = std::collections::HashMap::new();
    if !bridge.enabled() || accounts.is_empty() {
        return Ok(out);
    }
    let result = bridge
        .jmap("x:Account/get", json!({ "ids": accounts }))
        .await?;
    for item in result
        .get("list")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let used = item
            .get("usedDiskQuota")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = item
            .get("quotas")
            .and_then(|q| q.get("maxDiskQuota"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        out.insert(id.to_string(), (used, total));
    }
    Ok(out)
}

/// Set an account's mailbox disk quota in Stalwart (WS3.6). Written as the
/// pointer path Stalwart's patch semantics expect; a no-op update reports the
/// id as neither created nor updated, which we treat as success.
pub async fn set_account_quota(
    bridge: &MailBridge,
    account: &str,
    bytes: u64,
) -> Result<(), String> {
    if !bridge.enabled() {
        return Ok(());
    }
    let mut update = serde_json::Map::new();
    update.insert(account.to_string(), json!({ "quotas/maxDiskQuota": bytes }));
    let result = bridge
        .jmap("x:Account/set", json!({ "update": update }))
        .await?;
    if let Some(reason) = result
        .get("notUpdated")
        .and_then(|n| n.get(account))
        .filter(|v| !v.is_null())
    {
        return Err(format!("quota update rejected: {reason}"));
    }
    Ok(())
}

/// Permanently remove an account from Stalwart (WS5.4 erasure). An account
/// that is already gone counts as success.
pub async fn destroy_account(bridge: &MailBridge, account: &str) -> Result<(), String> {
    if !bridge.enabled() {
        return Ok(());
    }
    let result = bridge
        .jmap("x:Account/set", json!({ "destroy": [account] }))
        .await?;
    let destroyed = result
        .get("destroyed")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(account)));
    if destroyed {
        return Ok(());
    }
    if let Some(reason) = result.get("notDestroyed").and_then(|n| n.get(account)) {
        if reason.get("type").and_then(Value::as_str) == Some("notFound") {
            return Ok(());
        }
        return Err(format!("account destroy rejected: {reason}"));
    }
    Err(format!("account destroy: unexpected response {result}"))
}

/// Find the id of the most recent email whose `Message-ID` header equals
/// `expect` (with brackets), scanning the deepest `scan` emails account-wide.
/// Returns `None` until it lands — the send path polls this after SMTP,
/// letting Stalwart's delivery (Inbox or Junk) race with our lookup.
pub async fn find_by_message_id(
    bridge: &MailBridge,
    account: &str,
    expect: &str,
    scan: usize,
) -> Result<Option<String>, String> {
    let query = bridge
        .jmap_mail(
            "Email/query",
            json!({
                "accountId": account,
                "sort": [{ "property": "receivedAt", "isAscending": false }],
                "position": 0,
                "limit": scan
            }),
        )
        .await?;
    let ids: Vec<Value> = query
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(None);
    }
    let result = bridge
        .jmap_mail(
            "Email/get",
            json!({
                "accountId": account,
                "ids": ids,
                "properties": ["id", "header:Message-ID"]
            }),
        )
        .await?;
    for item in result
        .get("list")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let header = item
            .get("header:Message-ID")
            .and_then(Value::as_str)
            .map(|h| h.trim());
        if header == Some(expect) {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                return Ok(Some(id.to_string()));
            }
        }
    }
    Ok(None)
}

/// File the sender's self-copy into `sent_mailbox` and mark it read. Full-map
/// replacement (Stalwart 0.16 semantics) drops whatever Inbox/Junk produced.
pub async fn move_to_sent(
    bridge: &MailBridge,
    account: &str,
    email_id: &str,
    sent_mailbox: &str,
) -> Result<(), String> {
    let update = serde_json::Map::from_iter([(
        email_id.to_string(),
        json!({
            "mailboxIds": { sent_mailbox: true },
            "keywords": { "$seen": true }
        }),
    )]);
    let result = bridge
        .jmap_mail(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Fetch one map-valued property (e.g. `keywords`, `mailboxIds`) for a batch
/// of emails, keyed by email id. Stalwart returns the email as a map entry,
/// so we pass `properties: [property]` to keep responses small.
async fn current_map(
    bridge: &MailBridge,
    account: &str,
    email_ids: &[String],
    property: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let mut out = serde_json::Map::new();
    for chunk in email_ids.chunks(100) {
        let ids: Vec<Value> = chunk.iter().map(|s| json!(s)).collect();
        let result = bridge
            .jmap_mail(
                "Email/get",
                json!({
                    "accountId": account,
                    "ids": ids,
                    "properties": ["id", property]
                }),
            )
            .await?;
        if let Some(list) = result.get("list").and_then(Value::as_array) {
            for item in list {
                let id = item.get("id").and_then(Value::as_str);
                let val = item.get(property);
                if let (Some(id), Some(val)) = (id, val) {
                    out.insert(id.to_string(), val.clone());
                }
            }
        }
    }
    Ok(out)
}

/// Permanently delete emails (hard destroy; trash/soft-delete is a move).
pub async fn destroy(
    bridge: &MailBridge,
    account: &str,
    email_ids: &[String],
) -> Result<(), String> {
    if email_ids.is_empty() {
        return Ok(());
    }
    let result = bridge
        .jmap_mail(
            "Email/set",
            json!({ "accountId": account, "destroy": email_ids }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Resolve a blob (attachment) to its bytes for downloading.
pub async fn attachment_blob(
    bridge: &MailBridge,
    account: &str,
    blob_id: &str,
) -> Result<Option<Value>, String> {
    let result = bridge
        .jmap_mail(
            "Blob/get",
            json!({ "accountId": account, "ids": [blob_id] }),
        )
        .await?;
    // RFC 8620: each Blob/get list item identifies itself with `id`; missing
    // blobs appear in `notFound`.
    let item = result
        .get("list")
        .and_then(Value::as_array)
        .and_then(|a| a.first());
    match item {
        Some(item) if item.get("id").is_some() => Ok(Some(item.clone())),
        _ => Ok(None),
    }
}

async fn emails_by_id(
    bridge: &MailBridge,
    account: &str,
    ids: &[Value],
    props: &[&str],
    with_bodies: bool,
) -> Result<Vec<Value>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = serde_json::Map::new();
    args.insert("accountId".into(), json!(account));
    args.insert("ids".into(), json!(ids));
    args.insert("properties".into(), json!(props));
    if with_bodies {
        // RFC 8621: bodyValues is only populated on request. Fetch the decoded
        // text/HTML parts so readers can render without extra round-trips.
        args.insert("fetchTextBodyValues".into(), json!(true));
        args.insert("fetchHTMLBodyValues".into(), json!(true));
    }
    let result = bridge.jmap_mail("Email/get", Value::Object(args)).await?;
    Ok(result
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Flatten raw `Email/get` objects into the preview row contract (order
/// preserved, keywords collapsed to booleans).
fn to_rows(emails: Vec<Value>) -> Vec<Value> {
    emails
        .into_iter()
        .map(|e| {
            json!({
                "id": e["id"],
                "thread_id": e["threadId"],
                "subject": e.get("subject"),
                "date": e["date"],
                "received_at": e.get("receivedAt"),
                "from": e.get("from").and_then(Value::as_array).and_then(|a| a.first()),
                "to": e.get("to"),
                "cc": e.get("cc"),
                "bcc": e.get("bcc"),
                "preview": e.get("preview").and_then(Value::as_str).unwrap_or(""),
                "read": has_keyword(&e, "$seen"),
                "starred": has_keyword(&e, "$flagged"),
                "has_attachment": e.get("hasAttachment").and_then(Value::as_bool).unwrap_or(false),
                "size": e.get("size").and_then(Value::as_u64).unwrap_or(0),
                "mailboxes": e.get("mailboxIds"),
            })
        })
        .collect()
}

fn has_keyword(email: &Value, keyword: &str) -> bool {
    email
        .get("keywords")
        .and_then(Value::as_object)
        .map(|k| k.contains_key(keyword))
        .unwrap_or(false)
}

/// Concatenate decoded body part values for `textBody` or `htmlBody`.
fn body_for(email: &serde_json::Map<String, Value>, body_kind: &str) -> String {
    let mut out = String::new();
    let values = email.get("bodyValues").and_then(Value::as_object);
    let parts = email.get(body_kind).and_then(Value::as_array);
    if let (Some(values), Some(parts)) = (values, parts) {
        for part in parts {
            if let Some(part_id) = part.get("partId").and_then(Value::as_str) {
                if let Some(v) = values
                    .get(part_id)
                    .and_then(|x| x.get("value"))
                    .and_then(Value::as_str)
                {
                    out.push_str(v);
                }
            }
        }
    }
    out
}

/// Turn an `Email/set` response into an error if anything was rejected.
fn reject_set_failures(method: &str, result: Value) -> Result<(), String> {
    for container in ["notCreated", "notUpdated", "notDestroyed"] {
        if let Some(bad) = result.get(container).and_then(Value::as_array) {
            if !bad.is_empty() {
                return Err(format!(
                    "{method}: {container} on {bad:?} in {}",
                    result
                        .get("error")
                        .map_or_else(|| "response".to_string(), Value::to_string)
                ));
            }
        }
    }
    Ok(())
}

/// The `ok`/`fail`-style outcome keywords RFC 7489 §6.3 lets an authserv-id
/// emit. Anything else resolves to `"none"` so consumers never see raw text.
const AUTH_RESULT_KEYS: &[&str] = &[
    "pass",
    "fail",
    "softfail",
    "neutral",
    "none",
    "temperror",
    "permerror",
];

fn header_value(obj: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    match obj.get(name) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|v| v.as_str().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// Pull the first `method=verdict` token for the SPF/DKIM/DMARC triplet from
/// an `Authentication-Results` header. Missing or unknown methods come back
/// as `"none"`.
fn parse_auth_results(header: &str) -> (String, String, String) {
    let mut spf = String::from("none");
    let mut dkim = String::from("none");
    let mut dmarc = String::from("none");
    for segment in header.split(';') {
        let seg = segment.trim().to_lowercase();
        let parsed: Vec<&str> = seg.split_whitespace().collect();
        if parsed.is_empty() {
            continue;
        }
        // First token is `method=result`; a bare token (the authserv-id,
        // which only appears on the header's opening segment) is skipped.
        let Some((method, first_result)) = parsed[0].split_once('=') else {
            continue;
        };
        let verdict = first_result.trim_end_matches([';', ',']).trim().to_string();
        let verdict = if AUTH_RESULT_KEYS.contains(&verdict.as_str()) {
            verdict
        } else {
            String::from("none")
        };
        match method {
            "spf" => spf = verdict,
            "dkim" => dkim = verdict,
            "dmarc" => dmarc = verdict,
            _ => {}
        }
    }
    (spf, dkim, dmarc)
}

/// Best-effort spam score from the relay's banner headers (`X-Spam-Status:
/// Yes, score=3.1` or a bare `X-Spam-Score: 4.2`).
fn spam_score(obj: &serde_json::Map<String, Value>) -> Option<f64> {
    if let Some(status) = header_value(obj, "header:X-Spam-Status") {
        for chunk in status.split(',') {
            for token in chunk.split_whitespace() {
                if let Some(rest) = token.strip_prefix("score=") {
                    if let Ok(score) = rest.trim().parse::<f64>() {
                        return Some(score);
                    }
                }
            }
        }
    }
    header_value(obj, "header:X-Spam-Score").and_then(|s| s.trim().parse::<f64>().ok())
}

/// Real per-message deliverability/abuse verdicts (WS2.5). Sources, in order
/// of authority: the spam banners the relay stamps, then whether the message
/// sits in the Junk mailbox, then the SPF/DKIM/DMARC chain in
/// `Authentication-Results`.
fn security_verdicts(obj: &serde_json::Map<String, Value>, junk_mailbox: Option<&str>) -> Value {
    let (spf, dkim, dmarc) = obj
        .get("header:Authentication-Results")
        .or(obj.get("header:Received-SPF"))
        .and_then(Value::as_str)
        .map(parse_auth_results)
        .unwrap_or_else(|| {
            (
                String::from("none"),
                String::from("none"),
                String::from("none"),
            )
        });

    let score = spam_score(obj);
    let banner_spam = obj
        .get("header:X-Spam-Flag")
        .and_then(Value::as_str)
        .map(|f| f.eq_ignore_ascii_case("yes") || f.eq_ignore_ascii_case("true"));
    let status_spam = obj
        .get("header:X-Spam-Status")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase().starts_with("yes"));
    let in_junk = obj
        .get("mailboxIds")
        .and_then(Value::as_object)
        .map(|m| junk_mailbox.is_some_and(|junk| m.contains_key(junk)))
        .unwrap_or(false);
    let spam = banner_spam.unwrap_or(false) || status_spam.unwrap_or(false) || in_junk;

    json!({
        "spf": spf,
        "dkim": dkim,
        "dmarc": dmarc,
        "spam": spam,
        "score": score.unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_results_full_chain_parsed() {
        let (spf, dkim, dmarc) = parse_auth_results(
            "mail.crescentsphere.com; spf=pass smtp.mailfrom=bob@example.com; \
             dkim=pass header.i=@example.com; dmarc=pass action=none header.from=example.com",
        );
        assert_eq!(spf, "pass");
        assert_eq!(dkim, "pass");
        assert_eq!(dmarc, "pass");
    }

    #[test]
    fn auth_results_missing_method_is_none() {
        let (spf, dkim, dmarc) =
            parse_auth_results("mail.example.com; spf=fail smtp.mailfrom=spammer@evil.test");
        assert_eq!(spf, "fail");
        assert_eq!(dkim, "none");
        assert_eq!(dmarc, "none");
    }

    #[test]
    fn auth_results_unknown_verdict_normalised_to_none() {
        let (_, dkim, _) =
            parse_auth_results("mx1.example.com; dkim=broken reason=\"signature missing\"");
        assert_eq!(dkim, "none");
    }

    #[test]
    fn spam_verdict_flags() {
        let mut obj = serde_json::Map::new();
        obj.insert("header:X-Spam-Flag".into(), json!("YES"));
        let v = security_verdicts(&obj, None);
        assert!(v["spam"].as_bool().unwrap());

        let mut junk = serde_json::Map::new();
        junk.insert("mailboxIds".into(), json!({"abc": true, "def": true}));
        let v = security_verdicts(&junk, Some("def"));
        assert!(v["spam"].as_bool().unwrap());
    }

    #[test]
    fn spam_score_status_line() {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "header:X-Spam-Status".into(),
            json!("Yes, score=3.1 triggers=2.0 autolearn=ham version=3.4.1"),
        );
        assert_eq!(spam_score(&obj), Some(3.1));
    }

    #[test]
    fn no_signal_is_clean() {
        let obj = serde_json::Map::new();
        let v = security_verdicts(&obj, None);
        assert!(!v["spam"].as_bool().unwrap());
        assert_eq!(v["spf"], "none");
        assert_eq!(v["score"], 0.0);
    }
}
