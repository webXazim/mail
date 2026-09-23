//! Mail read/write path (WS2). Despite the historical filename, delivery is
//! NOT IMAP: it was redesigned to a single pinned admin credential calling the
//! Stalwart JMAP API (RFC 8620) on behalf of each user. Every method here runs
//! against the admin session with the target user's Stalwart account id in
//! `accountId` (admin impersonation) — we never store per-user mail secrets.
//!
//! Responsibilities: mailbox tree w/ counters, paginated thread previews,
//! full thread + bodies, read/starred flags, move, delete, advanced search, and
//! attachment blobs.

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::services::stalwart::StalwartService;

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

/// Compact mailbox roster with unread counts (sidebar). The snapshot also
/// carries the JMAP Mailbox/get state so clients can cheaply identify roster
/// changes without guessing from their locally-loaded message window.
pub async fn mailbox_snapshot(bridge: &StalwartService, account: &str) -> Result<Value, String> {
    let result = bridge
        .mail_read("Mailbox/get", json!({ "accountId": account }))
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
    Ok(json!({
        "mailboxes": out,
        "state": result.get("state").cloned().unwrap_or(Value::Null)
    }))
}

pub async fn mailboxes(bridge: &StalwartService, account: &str) -> Result<Value, String> {
    let snapshot = mailbox_snapshot(bridge, account).await?;
    Ok(snapshot.get("mailboxes").cloned().unwrap_or_else(|| Value::Array(Vec::new())))
}

/// Current Email/get state without downloading the mailbox. JMAP state tokens
/// drive multi-device realtime invalidation for flag/move/delete changes that
/// do not necessarily alter Inbox query membership.
pub async fn email_state(bridge: &StalwartService, account: &str) -> Result<Option<String>, String> {
    let result = bridge
        .mail_read(
            "Email/get",
            json!({ "accountId": account, "ids": [], "properties": ["id"] }),
        )
        .await?;
    Ok(result.get("state").and_then(Value::as_str).map(str::to_string))
}

pub async fn email_changes(
    bridge: &StalwartService,
    account: &str,
    since_state: &str,
    max_changes: usize,
) -> Result<Value, String> {
    let result = bridge
        .mail_read(
            "Email/changes",
            json!({
                "accountId": account,
                "sinceState": since_state,
                "maxChanges": max_changes.max(1).min(500) as i64
            }),
        )
        .await?;
    let created = result.get("created").and_then(Value::as_array).map_or(0, Vec::len);
    let updated = result.get("updated").and_then(Value::as_array).map_or(0, Vec::len);
    let destroyed = result.get("destroyed").and_then(Value::as_array).map_or(0, Vec::len);
    Ok(json!({
        "state": result.get("newState").cloned().unwrap_or_else(|| json!(since_state)),
        "has_more_changes": result.get("hasMoreChanges").and_then(Value::as_bool).unwrap_or(false),
        "changed": created + updated + destroyed > 0,
        "created": created,
        "updated": updated,
        "destroyed": destroyed
    }))
}

pub async fn mailbox_state(bridge: &StalwartService, account: &str) -> Result<Option<String>, String> {
    let result = bridge
        .mail_read(
            "Mailbox/get",
            json!({ "accountId": account, "ids": [], "properties": ["id"] }),
        )
        .await?;
    Ok(result.get("state").and_then(Value::as_str).map(str::to_string))
}

pub async fn mailbox_changes(
    bridge: &StalwartService,
    account: &str,
    since_state: &str,
    max_changes: usize,
) -> Result<Value, String> {
    let result = bridge
        .mail_read(
            "Mailbox/changes",
            json!({
                "accountId": account,
                "sinceState": since_state,
                "maxChanges": max_changes.max(1).min(500) as i64
            }),
        )
        .await?;
    let created = result.get("created").and_then(Value::as_array).map_or(0, Vec::len);
    let updated = result.get("updated").and_then(Value::as_array).map_or(0, Vec::len);
    let destroyed = result.get("destroyed").and_then(Value::as_array).map_or(0, Vec::len);
    Ok(json!({
        "state": result.get("newState").cloned().unwrap_or_else(|| json!(since_state)),
        "has_more_changes": result.get("hasMoreChanges").and_then(Value::as_bool).unwrap_or(false),
        "changed": created + updated + destroyed > 0
    }))
}

fn query_filter(scope: &str, mailbox: Option<&str>, trash: Option<&str>) -> Result<Option<Value>, String> {
    let not_trash = trash.map(|id| json!({
        "operator": "NOT",
        "conditions": [{ "inMailbox": id }]
    }));
    let combine = |mut parts: Vec<Value>| -> Option<Value> {
        parts.retain(|v| !v.is_null());
        match parts.len() {
            0 => None,
            1 => parts.into_iter().next(),
            _ => Some(json!({ "operator": "AND", "conditions": parts })),
        }
    };
    match scope {
        "mailbox" => mailbox
            .filter(|id| !id.is_empty())
            .map(|id| Some(json!({ "inMailbox": id })))
            .ok_or_else(|| "mailbox id is required".to_string()),
        "all" => Ok(not_trash),
        "unread" => Ok(combine(vec![not_trash.unwrap_or(Value::Null), json!({ "notKeyword": "$seen" })])),
        "starred" => Ok(combine(vec![not_trash.unwrap_or(Value::Null), json!({ "hasKeyword": "$flagged" })])),
        _ => Err(format!("unsupported mail query scope: {scope}")),
    }
}

/// Exact counters for virtual mailbox views that are not represented by one
/// physical JMAP Mailbox. These totals come from Email/query and therefore do
/// not depend on how many rows the browser has loaded.
pub async fn virtual_counts(
    bridge: &StalwartService,
    account: &str,
    trash: Option<&str>,
) -> Result<Value, String> {
    async fn total_for(
        bridge: &StalwartService,
        account: &str,
        filter: Option<Value>,
    ) -> Result<u64, String> {
        let mut args = json!({
            "accountId": account,
            "collapseThreads": true,
            "calculateTotal": true,
            "position": 0,
            "limit": 1
        });
        if let Some(filter) = filter { args["filter"] = filter; }
        let result = bridge.mail_read("Email/query", args).await?;
        Ok(result.get("total").and_then(Value::as_u64).unwrap_or(0))
    }

    let all = total_for(bridge, account, query_filter("all", None, trash)?).await?;
    let unread = total_for(bridge, account, query_filter("unread", None, trash)?).await?;
    let starred = total_for(bridge, account, query_filter("starred", None, trash)?).await?;
    Ok(json!({ "all": all, "unread": unread, "starred": starred }))
}

fn query_sort(sort: &str) -> Value {
    let (property, ascending) = match sort {
        "received_asc" => ("receivedAt", true),
        "sender_asc" => ("from", true),
        "sender_desc" => ("from", false),
        "subject_asc" => ("subject", true),
        "subject_desc" => ("subject", false),
        _ => ("receivedAt", false),
    };
    json!([{ "property": property, "isAscending": ascending }])
}

/// One stable page of collapsed conversation previews. The returned query
/// state belongs to the exact filter/sort used for the page. A caller can send
/// its previous state on the next request; if the membership/order changed in
/// between, `reset_required` tells the UI to restart from page one rather than
/// mixing two snapshots.
pub async fn thread_previews(
    bridge: &StalwartService,
    account: &str,
    scope: &str,
    in_mailbox: Option<&str>,
    limit: usize,
    anchor: Option<&str>,
    expected_query_state: Option<&str>,
    sort: &str,
    unread: bool,
    starred: bool,
    attachment: bool,
) -> Result<Value, String> {
    let trash = mailbox_id_for_role(bridge, account, "trash").await?;
    let mut filter = query_filter(scope, in_mailbox, trash.as_deref())?;
    let mut extras = Vec::new();
    if unread { extras.push(json!({ "notKeyword": "$seen" })); }
    if starred { extras.push(json!({ "hasKeyword": "$flagged" })); }
    if attachment { extras.push(json!({ "hasAttachment": true })); }
    if !extras.is_empty() {
        if let Some(base) = filter.take() { extras.insert(0, base); }
        filter = Some(if extras.len() == 1 { extras.remove(0) } else { json!({ "operator": "AND", "conditions": extras }) });
    }

    let mut args = json!({
        "accountId": account,
        "sort": query_sort(sort),
        "collapseThreads": true,
        "calculateTotal": true,
        "limit": limit as i64
    });
    if let Some(f) = filter { args["filter"] = f; }
    if let Some(a) = anchor {
        args["anchor"] = json!(a);
        args["anchorOffset"] = json!(1);
    } else {
        args["position"] = json!(0);
    }

    let result = match bridge.mail_read("Email/query", args.clone()).await {
        Ok(result) => (result, false),
        Err(err) if anchor.is_some() && err.to_string().to_ascii_lowercase().contains("anchor") => {
            // The anchor can disappear when another client moves/deletes mail
            // between page requests. Restart from position zero and signal the
            // browser to discard the mixed snapshot instead of surfacing a
            // transient pagination failure.
            if let Some(obj) = args.as_object_mut() {
                obj.remove("anchor");
                obj.remove("anchorOffset");
                obj.insert("position".into(), json!(0));
            }
            (bridge.mail_read("Email/query", args).await?, true)
        }
        Err(err) => return Err(err.to_string()),
    };
    let (result, anchor_reset) = result;
    let query_state = result.get("queryState").and_then(Value::as_str).unwrap_or("");
    let reset_required = anchor_reset || expected_query_state
        .filter(|s| !s.is_empty())
        .map(|s| s != query_state)
        .unwrap_or(false);
    let ids: Vec<Value> = result.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
    let position = result.get("position").and_then(Value::as_u64).unwrap_or(0);
    let total = result.get("total").and_then(Value::as_u64).unwrap_or(position + ids.len() as u64);
    let has_more = position.saturating_add(ids.len() as u64) < total;
    let next_anchor = ids.last().and_then(Value::as_str).map(str::to_string);
    let emails = emails_by_id(bridge, account, &ids, LIST_PROPS, false).await?;
    Ok(json!({
        "emails": to_rows(emails),
        "has_more": has_more,
        "next_anchor": next_anchor,
        "query_state": query_state,
        "reset_required": reset_required,
        "position": position,
        "total": total,
    }))
}

pub async fn mailbox_id_for_role(
    bridge: &StalwartService,
    account: &str,
    role: &str,
) -> Result<Option<String>, String> {
    let list = mailboxes(bridge, account).await?;
    for mb in list.as_array().into_iter().flatten() {
        if mb.get("role").and_then(Value::as_str) == Some(role) {
            return Ok(mb.get("id").and_then(Value::as_str).map(String::from));
        }
    }
    Ok(None)
}

pub async fn mailbox_by_id(
    bridge: &StalwartService,
    account: &str,
    mailbox_id: &str,
) -> Result<Option<Value>, String> {
    let result = bridge
        .mail_read("Mailbox/get", json!({ "accountId": account, "ids": [mailbox_id] }))
        .await?;
    Ok(result.get("list").and_then(Value::as_array).and_then(|v| v.first()).cloned())
}

pub async fn create_mailbox(
    bridge: &StalwartService,
    account: &str,
    name: &str,
    role: Option<&str>,
) -> Result<String, String> {
    let mut mailbox = json!({ "name": name, "isSubscribed": true });
    if let Some(role) = role { mailbox["role"] = json!(role); }
    let result = bridge
        .mail_write("Mailbox/set", json!({ "accountId": account, "create": { "new": mailbox } }))
        .await?;
    reject_set_failures("Mailbox/set", result.clone())?;
    result.get("created").and_then(|v| v.get("new")).and_then(|v| v.get("id"))
        .and_then(Value::as_str).map(str::to_string)
        .ok_or_else(|| "Mailbox/set did not return the created mailbox id".to_string())
}

pub async fn ensure_role_mailbox(
    bridge: &StalwartService,
    account: &str,
    role: &str,
    name: &str,
) -> Result<String, String> {
    if let Some(id) = mailbox_id_for_role(bridge, account, role).await? { return Ok(id); }
    create_mailbox(bridge, account, name, Some(role)).await
}

pub async fn rename_mailbox(
    bridge: &StalwartService,
    account: &str,
    mailbox_id: &str,
    name: &str,
) -> Result<(), String> {
    let mut update = serde_json::Map::new();
    update.insert(mailbox_id.to_string(), json!({ "name": name }));
    let result = bridge.mail_write("Mailbox/set", json!({
        "accountId": account,
        "update": update
    })).await?;
    reject_set_failures("Mailbox/set", result)
}

pub async fn destroy_mailbox(
    bridge: &StalwartService,
    account: &str,
    mailbox_id: &str,
) -> Result<(), String> {
    let result = bridge.mail_write("Mailbox/set", json!({
        "accountId": account,
        "destroy": [mailbox_id],
        "onDestroyRemoveEmails": false
    })).await?;
    reject_set_failures("Mailbox/set", result)
}

/// Permanently remove every message currently in one mailbox. Querying from
/// position zero after each successful batch avoids anchor invalidation while
/// the result set shrinks. This is intentionally used only for explicit
/// destructive operations such as Empty Trash.
pub async fn empty_mailbox(
    bridge: &StalwartService,
    account: &str,
    mailbox_id: &str,
) -> Result<usize, String> {
    let mut deleted = 0usize;
    for _ in 0..10_000 {
        let result = bridge.mail_read("Email/query", json!({
            "accountId": account,
            "filter": { "inMailbox": mailbox_id },
            "collapseThreads": false,
            "position": 0,
            "limit": 100
        })).await?;
        let ids = result.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
        if ids.is_empty() { return Ok(deleted); }
        let batch: Vec<String> = ids.iter().filter_map(Value::as_str).map(str::to_string).collect();
        if batch.is_empty() { return Ok(deleted); }
        destroy(bridge, account, &batch).await?;
        deleted += batch.len();
    }
    Err("empty mailbox exceeded safety iteration limit".to_string())
}

/// Full thread (conversation) with every message's bodies and headers.
pub async fn thread(bridge: &StalwartService, account: &str, thread_id: &str) -> Result<Value, String> {
    let t = bridge
        .mail_read(
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

#[derive(Debug)]
pub enum SearchError {
    Invalid(String),
    Store(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::Store(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchTerm {
    negated: bool,
    key: Option<String>,
    value: String,
}

fn tokenize_search(query: &str) -> Result<Vec<String>, SearchError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in query.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quoted {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if ch.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if escaped || quoted {
        return Err(SearchError::Invalid("Search query contains an unterminated quoted value".into()));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn parse_search_terms(query: &str) -> Result<Vec<SearchTerm>, SearchError> {
    if query.chars().count() > 2048 {
        return Err(SearchError::Invalid("Search query is too long".into()));
    }
    let tokens = tokenize_search(query)?;
    if tokens.len() > 64 {
        return Err(SearchError::Invalid("Search query has too many terms".into()));
    }
    let mut terms = Vec::new();
    for token in tokens {
        if token.chars().count() > 512 {
            return Err(SearchError::Invalid("A search term is too long".into()));
        }
        let (negated, raw) = token
            .strip_prefix('-')
            .filter(|rest| !rest.is_empty())
            .map(|rest| (true, rest))
            .unwrap_or((false, token.as_str()));
        let (key, value) = match raw.split_once(':') {
            Some((candidate, value)) if matches!(candidate.to_ascii_lowercase().as_str(),
                "from" | "to" | "cc" | "bcc" | "subject" | "body" | "has" | "is" | "in" |
                "after" | "before" | "newer" | "older" | "newer_than" | "older_than" | "label") =>
            {
                (Some(candidate.to_ascii_lowercase()), value)
            }
            _ => (None, raw),
        };
        if key.is_some() && value.trim().is_empty() {
            return Err(SearchError::Invalid(format!("Search operator {raw:?} is missing a value")));
        }
        terms.push(SearchTerm {
            negated,
            key,
            value: value.trim().to_string(),
        });
    }
    Ok(terms)
}

fn negate_filter(filter: Value, negated: bool) -> Value {
    if negated {
        json!({ "operator": "NOT", "conditions": [filter] })
    } else {
        filter
    }
}

fn parse_search_date(value: &str, operator: &str, timezone_offset_minutes: i32) -> Result<String, SearchError> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let local_midnight = date.and_hms_opt(0, 0, 0).expect("valid midnight");
        let dt = Utc.from_utc_datetime(&local_midnight)
            - Duration::minutes(timezone_offset_minutes as i64);
        return Ok(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(dt.with_timezone(&Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    Err(SearchError::Invalid(format!(
        "{operator}: expects YYYY-MM-DD or an RFC 3339 timestamp"
    )))
}

fn parse_relative_duration(value: &str, operator: &str) -> Result<Duration, SearchError> {
    let split = value
        .find(|ch: char| !ch.is_ascii_digit())
        .ok_or_else(|| SearchError::Invalid(format!("{operator}: expects a value such as 7d, 12h, 2w, 3m or 1y")))?;
    let (amount, unit) = value.split_at(split);
    let amount: i64 = amount
        .parse()
        .map_err(|_| SearchError::Invalid(format!("{operator}: has an invalid duration")))?;
    if amount <= 0 || amount > 100_000 || unit.len() != 1 {
        return Err(SearchError::Invalid(format!("{operator}: has an invalid duration")));
    }
    let duration = match unit.to_ascii_lowercase().as_str() {
        "h" => Duration::hours(amount),
        "d" => Duration::days(amount),
        "w" => Duration::weeks(amount),
        // Search durations are intentionally fixed periods rather than calendar
        // arithmetic, matching the behavior users expect from mail search.
        "m" => Duration::days(amount.saturating_mul(30)),
        "y" => Duration::days(amount.saturating_mul(365)),
        _ => return Err(SearchError::Invalid(format!("{operator}: uses an unsupported duration unit"))),
    };
    Ok(duration)
}

fn role_for_search_scope(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "inbox" => Some("inbox"),
        "sent" => Some("sent"),
        "trash" => Some("trash"),
        "spam" | "junk" => Some("junk"),
        "archive" | "archived" => Some("archive"),
        _ => None,
    }
}

async fn resolve_search_mailbox(
    bridge: &StalwartService,
    account: &str,
    value: &str,
) -> Result<String, SearchError> {
    if let Some(role) = role_for_search_scope(value) {
        return mailbox_id_for_role(bridge, account, role)
            .await
            .map_err(SearchError::Store)?
            .ok_or_else(|| SearchError::Invalid(format!("Mailbox {value:?} does not exist")));
    }

    let roster = mailboxes(bridge, account).await.map_err(SearchError::Store)?;
    let needle = value.trim().to_lowercase();
    let mut matches = roster
        .as_array()
        .into_iter()
        .flatten()
        .filter(|mailbox| mailbox.get("name").and_then(Value::as_str).map(|name| name.to_lowercase() == needle).unwrap_or(false))
        .filter_map(|mailbox| mailbox.get("id").and_then(Value::as_str).map(str::to_string));
    let Some(first) = matches.next() else {
        return Err(SearchError::Invalid(format!("Mailbox {value:?} was not found")));
    };
    if matches.next().is_some() {
        return Err(SearchError::Invalid(format!("Mailbox name {value:?} is ambiguous")));
    }
    Ok(first)
}

async fn compile_search_filter(
    bridge: &StalwartService,
    account: &str,
    query: &str,
    timezone_offset_minutes: i32,
) -> Result<Value, SearchError> {
    let terms = parse_search_terms(query)?;
    if terms.is_empty() {
        return Err(SearchError::Invalid("Search query is required".into()));
    }

    let has_explicit_scope = terms.iter().any(|term| {
        if term.negated {
            return false;
        }
        let value = term.value.to_ascii_lowercase();
        match term.key.as_deref() {
            Some("in") => !matches!(value.as_str(), "unread" | "starred"),
            Some("is") => matches!(value.as_str(), "sent" | "trash" | "spam" | "junk" | "archive" | "archived"),
            _ => false,
        }
    });
    let mut conditions = Vec::new();

    // Normal searches intentionally exclude Trash and Spam, matching the
    // existing advanced-search scope. `in:all`/`in:any` is the explicit escape.
    if !has_explicit_scope {
        for role in ["trash", "junk"] {
            if let Some(id) = mailbox_id_for_role(bridge, account, role).await.map_err(SearchError::Store)? {
                conditions.push(json!({ "operator": "NOT", "conditions": [{ "inMailbox": id }] }));
            }
        }
    }

    for term in terms {
        let key = term.key.as_deref();
        let value = term.value.as_str();
        let filter = match key {
            None => json!({ "text": value }),
            Some("from") => json!({ "from": value }),
            Some("to") => json!({ "to": value }),
            Some("cc") => json!({ "cc": value }),
            Some("bcc") => json!({ "bcc": value }),
            Some("subject") => json!({ "subject": value }),
            Some("body") => json!({ "body": value }),
            Some("has") if value.eq_ignore_ascii_case("attachment") => json!({ "hasAttachment": true }),
            Some("has") => return Err(SearchError::Invalid(format!("Unsupported has: operator value {value:?}"))),
            Some("is") => match value.to_ascii_lowercase().as_str() {
                "unread" => json!({ "notKeyword": "$seen" }),
                "read" => json!({ "hasKeyword": "$seen" }),
                "starred" => json!({ "hasKeyword": "$flagged" }),
                "attachment" => json!({ "hasAttachment": true }),
                "sent" | "trash" | "spam" | "junk" | "archive" | "archived" => {
                    let id = resolve_search_mailbox(bridge, account, value).await?;
                    json!({ "inMailbox": id })
                }
                "draft" | "drafts" => return Err(SearchError::Invalid("Drafts are application-owned and are not part of mailbox search".into())),
                other => return Err(SearchError::Invalid(format!("Unsupported is: operator value {other:?}"))),
            },
            Some("in") => match value.to_ascii_lowercase().as_str() {
                "all" | "any" | "everything" => continue,
                "unread" => json!({ "notKeyword": "$seen" }),
                "starred" => json!({ "hasKeyword": "$flagged" }),
                "draft" | "drafts" => return Err(SearchError::Invalid("Drafts are application-owned and are not part of mailbox search".into())),
                _ => {
                    let id = resolve_search_mailbox(bridge, account, value).await?;
                    json!({ "inMailbox": id })
                }
            },
            Some("after") | Some("newer") => json!({ "after": parse_search_date(value, key.unwrap(), timezone_offset_minutes)? }),
            Some("before") | Some("older") => json!({ "before": parse_search_date(value, key.unwrap(), timezone_offset_minutes)? }),
            Some("newer_than") => {
                let boundary = Utc::now() - parse_relative_duration(value, "newer_than")?;
                json!({ "after": boundary.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) })
            }
            Some("older_than") => {
                let boundary = Utc::now() - parse_relative_duration(value, "older_than")?;
                json!({ "before": boundary.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) })
            }
            Some("label") => return Err(SearchError::Invalid("label: search is not server-backed yet".into())),
            Some(other) => return Err(SearchError::Invalid(format!("Unsupported search operator {other:?}"))),
        };
        conditions.push(negate_filter(filter, term.negated));
    }

    match conditions.len() {
        0 => Ok(json!({})),
        1 => Ok(conditions.remove(0)),
        _ => Ok(json!({ "operator": "AND", "conditions": conditions })),
    }
}

/// Server-authoritative advanced mail search. The query language is parsed by
/// CS Mail and compiled to RFC 8621 Email/query filters; result membership,
/// ordering and total counts therefore come from Stalwart's search index, not
/// from the browser's currently-loaded mailbox window.
pub async fn search(
    bridge: &StalwartService,
    account: &str,
    query: &str,
    limit: usize,
    anchor: Option<&str>,
    expected_query_state: Option<&str>,
    sort: &str,
    timezone_offset_minutes: i32,
) -> Result<Value, SearchError> {
    let timezone_offset_minutes = timezone_offset_minutes.clamp(-14 * 60, 14 * 60);
    let filter = compile_search_filter(bridge, account, query, timezone_offset_minutes).await?;
    let mut args = json!({
        "accountId": account,
        "filter": filter,
        "sort": query_sort(sort),
        "collapseThreads": true,
        "calculateTotal": true,
        "limit": limit as i64
    });
    if let Some(anchor) = anchor.filter(|value| !value.is_empty()) {
        args["anchor"] = json!(anchor);
        args["anchorOffset"] = json!(1);
    } else {
        args["position"] = json!(0);
    }

    let result = match bridge.mail_read("Email/query", args.clone()).await {
        Ok(result) => (result, false),
        Err(err) if anchor.is_some() && err.to_string().to_ascii_lowercase().contains("anchor") => {
            if let Some(object) = args.as_object_mut() {
                object.remove("anchor");
                object.remove("anchorOffset");
                object.insert("position".into(), json!(0));
            }
            (bridge.mail_read("Email/query", args).await.map_err(|error| SearchError::Store(error.to_string()))?, true)
        }
        Err(err) => return Err(SearchError::Store(err.to_string())),
    };
    let (result, anchor_reset) = result;
    let query_state = result.get("queryState").and_then(Value::as_str).unwrap_or("");
    let reset_required = anchor_reset
        || expected_query_state
            .filter(|state| !state.is_empty())
            .map(|state| state != query_state)
            .unwrap_or(false);
    let ids: Vec<Value> = result.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
    let position = result.get("position").and_then(Value::as_u64).unwrap_or(0);
    let total = result.get("total").and_then(Value::as_u64).unwrap_or(position + ids.len() as u64);
    let has_more = position.saturating_add(ids.len() as u64) < total;
    let next_anchor = ids.last().and_then(Value::as_str).map(str::to_string);
    let emails = emails_by_id(bridge, account, &ids, LIST_PROPS, false)
        .await
        .map_err(SearchError::Store)?;
    Ok(json!({
        "emails": to_rows(emails),
        "has_more": has_more,
        "next_anchor": next_anchor,
        "query_state": query_state,
        "reset_required": reset_required,
        "position": position,
        "total": total
    }))
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn tokenizer_supports_quoted_operator_values_and_negation() {
        let terms = parse_search_terms(r#"from:alice@example.com subject:"Quarterly report" -has:attachment plain"#).unwrap();
        assert_eq!(terms.len(), 4);
        assert_eq!(terms[0].key.as_deref(), Some("from"));
        assert_eq!(terms[1].value, "Quarterly report");
        assert!(terms[2].negated);
        assert_eq!(terms[3].key, None);
        let url = parse_search_terms("https://example.com").unwrap();
        assert_eq!(url[0].key, None);
        assert_eq!(url[0].value, "https://example.com");
    }

    #[test]
    fn tokenizer_rejects_unterminated_quotes() {
        assert!(matches!(parse_search_terms("subject:\"open"), Err(SearchError::Invalid(_))));
    }

    #[test]
    fn date_only_search_uses_client_timezone_offset() {
        assert_eq!(
            parse_search_date("2026-09-21", "after", 180).unwrap(),
            "2026-09-20T21:00:00Z"
        );
    }

    #[test]
    fn relative_search_duration_accepts_mail_style_units() {
        assert_eq!(parse_relative_duration("7d", "newer_than").unwrap(), Duration::days(7));
        assert_eq!(parse_relative_duration("2w", "newer_than").unwrap(), Duration::weeks(2));
        assert!(parse_relative_duration("today", "newer_than").is_err());
        assert!(parse_relative_duration("999999999d", "newer_than").is_err());
    }
}

/// Mark a set of emails read/unread (`$seen`) and/or starred (`$flagged`).
/// Stalwart 0.16's `Email/set` expects a full `keywords` map, not the RFC 8621
/// `$is`/`$not` patch, so we merge with the current keywords first.
pub async fn set_flags(
    bridge: &StalwartService,
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
        .mail_write(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Move emails into `to`; when the UI knows the current mailbox it passes
/// `from` so this is a true move. We send a full mailbox membership map
/// rather than relying on patch semantics for this provider operation.
pub async fn move_emails(
    bridge: &StalwartService,
    account: &str,
    email_ids: &[String],
    from: Option<&str>,
    from_by_email: Option<&HashMap<String, String>>,
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
        if let Some(f) = from_by_email
            .and_then(|items| items.get(&id))
            .map(String::as_str)
            .or(from)
        {
            mailboxes.remove(f);
        }
        mailboxes.insert(to.into(), json!(true));
        update.insert(id.clone(), json!({ "mailboxIds": mailboxes }));
    }
    if update.is_empty() {
        return Ok(());
    }
    let result = bridge
        .mail_write(
            "Email/set",
            json!({ "accountId": account, "update": update }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Id of the `Sent` folder for an account (from the mailbox roster), so the
/// send path can file the sender's self-copy exactly where webmail shows it.
pub async fn sent_mailbox_id(bridge: &StalwartService, account: &str) -> Result<Option<String>, String> {
    mailbox_id_for_role(bridge, account, "sent").await
}

/// Id of the `Junk` folder, used to class a message as spam even when the
/// relay stamps no explicit spam header (WS2.5).
pub async fn junk_mailbox_id(bridge: &StalwartService, account: &str) -> Result<Option<String>, String> {
    mailbox_id_for_role(bridge, account, "junk").await
}

/// Id of the `Inbox` folder (realtime new-mail poller, WS2.6).
pub async fn inbox_mailbox_id(
    bridge: &StalwartService,
    account: &str,
) -> Result<Option<String>, String> {
    mailbox_id_for_role(bridge, account, "inbox").await
}

/// Current JMAP Email/query state for the Inbox query used by realtime. The
/// state token, rather than a timestamp, lets a multi-instance worker resume
/// without dropping messages that share the same receivedAt value.
pub async fn inbox_query_state(
    bridge: &StalwartService,
    account: &str,
) -> Result<Option<String>, String> {
    let Some(inbox) = inbox_mailbox_id(bridge, account).await? else {
        return Ok(None);
    };
    let result = bridge
        .mail_read(
            "Email/query",
            json!({
                "accountId": account,
                "filter": { "inMailbox": inbox },
                "sort": [{ "property": "receivedAt", "isAscending": false }],
                "collapseThreads": false,
                "position": 0,
                "limit": 1
            }),
        )
        .await?;
    Ok(result
        .get("queryState")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Incremental Inbox membership changes from a prior Email/query state.
/// `Email/queryChanges` gives us exact added ids and a new resume token, which
/// is substantially safer than polling `receivedAt > timestamp`.
pub async fn inbox_query_changes(
    bridge: &StalwartService,
    account: &str,
    since_query_state: &str,
    max_changes: usize,
) -> Result<Value, String> {
    let Some(inbox) = inbox_mailbox_id(bridge, account).await? else {
        return Ok(json!({
            "query_state": since_query_state,
            "has_more_changes": false,
            "emails": []
        }));
    };
    let result = bridge
        .mail_read(
            "Email/queryChanges",
            json!({
                "accountId": account,
                "filter": { "inMailbox": inbox },
                "sort": [{ "property": "receivedAt", "isAscending": false }],
                "collapseThreads": false,
                "sinceQueryState": since_query_state,
                "maxChanges": max_changes.max(1).min(500) as i64
            }),
        )
        .await?;

    let ids: Vec<Value> = result
        .get("added")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").cloned())
        .collect();
    let emails = if ids.is_empty() {
        Vec::new()
    } else {
        emails_by_id(bridge, account, &ids, LIST_PROPS, false).await?
    };

    let removed = result.get("removed").and_then(Value::as_array).map_or(0, Vec::len);
    let added = ids.len();
    Ok(json!({
        "query_state": result
            .get("newQueryState")
            .cloned()
            .unwrap_or_else(|| json!(since_query_state)),
        "has_more_changes": result
            .get("hasMoreChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "changed": added + removed > 0,
        "removed": removed,
        "emails": to_rows(emails)
    }))
}

/// Inbox messages received after `since` (RFC 3339), newest first, as preview
/// rows. `since = None` means "give me the newest mail" — retained for
/// compatibility with older diagnostics; realtime now uses query-state deltas.
pub async fn recent_inbox(
    bridge: &StalwartService,
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
        .mail_read(
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

/// Find the id of the most recent email whose `Message-ID` header equals
/// `expect` (with brackets), scanning the deepest `scan` emails account-wide.
/// Returns `None` until it lands — the send path polls this after SMTP,
/// letting Stalwart's delivery (Inbox or Junk) race with our lookup.
pub async fn find_by_message_id(
    bridge: &StalwartService,
    account: &str,
    expect: &str,
    scan: usize,
) -> Result<Option<String>, String> {
    let query = bridge
        .mail_read(
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
        .mail_read(
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
    bridge: &StalwartService,
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
        .mail_write(
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
    bridge: &StalwartService,
    account: &str,
    email_ids: &[String],
    property: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let mut out = serde_json::Map::new();
    for chunk in email_ids.chunks(100) {
        let ids: Vec<Value> = chunk.iter().map(|s| json!(s)).collect();
        let result = bridge
            .mail_read(
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
    bridge: &StalwartService,
    account: &str,
    email_ids: &[String],
) -> Result<(), String> {
    if email_ids.is_empty() {
        return Ok(());
    }
    let result = bridge
        .mail_write(
            "Email/set",
            json!({ "accountId": account, "destroy": email_ids }),
        )
        .await?;
    reject_set_failures("Email/set", result)
}

/// Resolve a blob (attachment) to its bytes for downloading.
pub async fn attachment_blob(
    bridge: &StalwartService,
    account: &str,
    blob_id: &str,
) -> Result<Option<Value>, String> {
    let result = bridge
        .mail_read(
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
    bridge: &StalwartService,
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
    let result = bridge.mail_read("Email/get", Value::Object(args)).await?;
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
                "keywords": e.get("keywords"),
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
        let Some(bad) = result.get(container) else { continue; };
        let non_empty = bad.as_object().map(|v| !v.is_empty())
            .or_else(|| bad.as_array().map(|v| !v.is_empty()))
            .unwrap_or(false);
        if non_empty {
            return Err(format!("{method}: {container}: {bad}"));
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
    #[test]
    fn virtual_scope_filters_exclude_trash() {
        let all = query_filter("all", None, Some("trash-id")).unwrap().unwrap();
        assert_eq!(all["operator"], "NOT");
        assert_eq!(all["conditions"][0]["inMailbox"], "trash-id");

        let unread = query_filter("unread", None, Some("trash-id")).unwrap().unwrap();
        assert_eq!(unread["operator"], "AND");
        assert!(unread["conditions"].as_array().unwrap().iter().any(|v| v.get("notKeyword") == Some(&json!("$seen"))));
    }

    #[test]
    fn mailbox_scope_requires_id() {
        assert!(query_filter("mailbox", None, None).is_err());
        assert_eq!(query_filter("mailbox", Some("m1"), None).unwrap().unwrap()["inMailbox"], "m1");
    }

    #[test]
    fn set_failures_accept_jmap_object_shape() {
        assert!(reject_set_failures("Mailbox/set", json!({"notDestroyed": {}})).is_ok());
        assert!(reject_set_failures("Mailbox/set", json!({
            "notDestroyed": {"m1": {"type": "mailboxHasEmail"}}
        })).unwrap_err().contains("mailboxHasEmail"));
    }

}
