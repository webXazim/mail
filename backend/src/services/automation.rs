//! Upgrade 12 incoming-mail automation.
//!
//! PostgreSQL stores the desired rules/forwarding/vacation state. The service
//! compiles that state into one CS Mail-owned Sieve script and reconciles it
//! to the user's Stalwart account through JMAP for Sieve (RFC 9661). Mutations
//! mark a revision dirty before provider IO, so an upstream outage cannot lose
//! the user's intent; the background worker retries until applied.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::types::Json;
use uuid::Uuid;

use crate::services::{entitlements, imap};
use crate::services::stalwart::StalwartError;
use crate::state::AppState;

const MANAGED_SCRIPT_NAME: &str = "cs-mail-managed";
const LEGACY_MANAGED_SCRIPT_NAME: &str = "cs-mailer-managed";
const MAX_RULES: usize = 100;
const MAX_CONDITIONS: usize = 12;
const MAX_ACTIONS: usize = 12;
const MAX_RULE_VALUE: usize = 1024;

#[derive(sqlx::FromRow, Clone)]
struct RuleRow {
    id: Uuid,
    name: String,
    enabled: bool,
    position: i32,
    conditions: Json<Value>,
    actions: Json<Value>,
}

#[derive(sqlx::FromRow, Clone)]
struct ForwardRow {
    enabled: bool,
    target_email: String,
    keep_copy: bool,
    verified_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(sqlx::FromRow, Clone)]
struct VacationRow {
    enabled: bool,
    subject: String,
    message: String,
    only_contacts: bool,
    starts_at: Option<NaiveDate>,
    ends_at: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub status: &'static str,
    pub revision: i64,
    pub script_id: Option<String>,
}

fn provider_error(error: StalwartError) -> String {
    match error {
        StalwartError::Rejected { operation, kind, description } => {
            format!("{operation}: {kind}: {description}")
        }
        other => other.to_string(),
    }
}

pub async fn mark_dirty(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<i64, String> {
    let revision: i64 = sqlx::query_scalar(
        "INSERT INTO mail_automation_state (user_id, mailbox_id, desired_revision, status, next_retry_at, updated_at)
         VALUES ($1, $2, 1, 'pending', now(), now())
         ON CONFLICT (mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE
         SET desired_revision = mail_automation_state.desired_revision + 1,
             status = 'pending', last_error = '', next_retry_at = now(), updated_at = now()
         RETURNING desired_revision",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(revision)
}

async fn account_for(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<(String, String), String> {
    let row: Option<(String, Option<String>, String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT m.address::text,m.provider_account_id,m.local_part,m.provider_marker,d.provider_domain_id,d.is_system
         FROM mailboxes m
         JOIN organization_domains d ON d.id=m.domain_id
         JOIN organizations o ON o.id=m.organization_id AND o.status='active'
         JOIN organization_memberships om ON om.organization_id=m.organization_id AND om.user_id=$1 AND om.status='active'
         WHERE m.id=$2 AND m.user_id=$1 AND m.deleted_at IS NULL AND m.status='active'",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let (email,cached,local,marker,provider_domain_id,is_system) = row.ok_or_else(|| "business mailbox is not active".to_string())?;
    if !state.stalwart.enabled() { return Err("mail service is not configured".to_string()); }
    let account = if is_system {
        state.stalwart.find_account_by_email(&email).await.map_err(provider_error)?
    } else {
        let domain_id=provider_domain_id.as_deref().filter(|v|!v.trim().is_empty()).ok_or_else(||"mailbox provider domain binding is missing".to_string())?;
        state.stalwart.find_customer_account(domain_id,&local,&marker).await.map_err(provider_error)?
    }.ok_or_else(|| "mailbox is not provisioned".to_string())?;
    if let Some(cached)=cached.filter(|value| !value.trim().is_empty()) {
        if cached != account { return Err("stored provider account id does not match mailbox ownership".into()); }
    } else {
        let _ = sqlx::query("UPDATE mailboxes SET provider_account_id=$1,sync_status='ready',sync_error='',updated_at=now() WHERE id=$2")
            .bind(&account).bind(mailbox_id).execute(&state.db).await;
    }
    Ok((email, account))
}

async fn desired_state(
    state: &AppState,
    mailbox_id: Uuid,
) -> Result<(Vec<RuleRow>, Option<ForwardRow>, Option<VacationRow>, Vec<String>), String> {
    let rules: Vec<RuleRow> = sqlx::query_as(
        "SELECT id, name, enabled, position, conditions, actions
         FROM mail_rules WHERE mailbox_id = $1 ORDER BY position, created_at, id",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let forwarding: Option<ForwardRow> = sqlx::query_as(
        "SELECT enabled, target_email::text AS target_email, keep_copy, verified_at
         FROM mail_forwarding WHERE mailbox_id = $1",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let vacation: Option<VacationRow> = sqlx::query_as(
        "SELECT enabled, subject, message, only_contacts, starts_at, ends_at
         FROM mail_vacation WHERE mailbox_id = $1",
    )
    .bind(mailbox_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    let contacts: Vec<String> = sqlx::query_scalar(
        "SELECT lower(email) FROM contacts WHERE mailbox_id = $1 ORDER BY lower(email) LIMIT 1000",
    )
    .bind(mailbox_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok((rules, forwarding, vacation, contacts))
}

fn quote(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => {},
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {},
            c => out.push(c),
        }
    }
    out
}

fn multiline_string(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::from("text:\r\n");
    for line in normalized.split('\n') {
        if line.starts_with('.') { out.push('.'); }
        for ch in line.chars() {
            if ch == '\t' || !ch.is_control() { out.push(ch); }
        }
        out.push_str("\r\n");
    }
    out.push_str(".\r\n");
    out
}

fn keyword_for_label(label: &str) -> String {
    let mut slug = String::from("cs-label-");
    for ch in label.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if matches!(ch, '-' | '_') || ch.is_whitespace() {
            if !slug.ends_with('-') { slug.push('-'); }
        }
        if slug.len() >= 56 { break; }
    }
    while slug.ends_with('-') { slug.pop(); }
    if slug == "cs-label" || slug == "cs-label-" { "cs-label-mail".into() } else { slug }
}

pub fn validate_rule(name: &str, conditions: &Value, actions: &Value) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err("Rule name must be between 1 and 120 printable characters".into());
    }
    let conditions = conditions.as_array().ok_or("conditions must be an array")?;
    let actions = actions.as_array().ok_or("actions must be an array")?;
    if conditions.is_empty() || conditions.len() > MAX_CONDITIONS {
        return Err(format!("A rule must have 1 to {MAX_CONDITIONS} conditions"));
    }
    if actions.is_empty() || actions.len() > MAX_ACTIONS {
        return Err(format!("A rule must have 1 to {MAX_ACTIONS} actions"));
    }
    for condition in conditions {
        let obj = condition.as_object().ok_or("Each condition must be an object")?;
        let field = obj.get("field").and_then(Value::as_str).ok_or("Condition field is required")?;
        match field {
            "from" | "to" | "subject" => {
                let value = obj.get("value").and_then(Value::as_str).unwrap_or("").trim();
                if value.is_empty() || value.len() > MAX_RULE_VALUE { return Err(format!("Invalid {field} condition")); }
            }
            "hasAttachment" => {}
            "size" => {
                let op = obj.get("op").and_then(Value::as_str).unwrap_or("");
                let size = obj.get("size").and_then(Value::as_u64).unwrap_or(0);
                if !matches!(op, "larger" | "smaller") || size == 0 || size > 10_000_000 {
                    return Err("Invalid size condition".into());
                }
            }
            "date" => {
                let op = obj.get("op").and_then(Value::as_str).unwrap_or("");
                let value = obj.get("value").and_then(Value::as_str).unwrap_or("");
                if !matches!(op, "before" | "after" | "on" | "not-on" | "on-or-before" | "on-or-after")
                    || NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
                    return Err("Invalid date condition".into());
                }
            }
            _ => return Err(format!("Unsupported condition field '{field}'")),
        }
    }
    for action in actions {
        let obj = action.as_object().ok_or("Each action must be an object")?;
        let kind = obj.get("kind").and_then(Value::as_str).ok_or("Action kind is required")?;
        match kind {
            "label" | "move" => {
                let value = obj.get("value").and_then(Value::as_str).unwrap_or("").trim();
                if value.is_empty() || value.len() > 255 { return Err(format!("Invalid {kind} action")); }
            }
            "forward" => {
                let value = obj.get("value").and_then(Value::as_str).unwrap_or("").trim();
                if !valid_email(value) { return Err("Forward action requires a valid email address".into()); }
            }
            "archive" | "keep-in-inbox" | "mark-read" | "mark-starred" | "discard" => {}
            _ => return Err(format!("Unsupported action '{kind}'")),
        }
    }
    Ok(())
}

pub fn valid_email(value: &str) -> bool {
    let value = value.trim();
    if value.len() > 320 || value.chars().any(|c| c.is_control() || c.is_whitespace()) { return false; }
    let Some((local, domain)) = value.rsplit_once('@') else { return false; };
    !local.is_empty() && !domain.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn condition_to_sieve(value: &Value, requires: &mut BTreeSet<&'static str>) -> Result<String, String> {
    let obj = value.as_object().ok_or("condition must be an object")?;
    let field = obj.get("field").and_then(Value::as_str).unwrap_or("");
    match field {
        "from" => Ok(format!("address :contains \"From\" \"{}\"", quote(obj["value"].as_str().unwrap_or("")))),
        "to" => Ok(format!("address :contains [\"To\", \"Cc\"] \"{}\"", quote(obj["value"].as_str().unwrap_or("")))),
        "subject" => Ok(format!("header :contains \"Subject\" \"{}\"", quote(obj["value"].as_str().unwrap_or("")))),
        "hasAttachment" => {
            // RFC 5703 :anychild examines all nested MIME body parts, so this
            // is a real attachment-metadata test rather than a body-text guess.
            requires.insert("mime");
            Ok(concat!(
                "anyof(",
                "header :mime :anychild :param \"filename\" :matches [\"Content-Type\", \"Content-Disposition\"] \"*\", ",
                "header :mime :anychild :param \"name\" :matches \"Content-Type\" \"*\", ",
                "header :mime :anychild :matches \"Content-Disposition\" \"attachment*\"",
                ")"
            ).into())
        }
        "size" => {
            let bytes = obj.get("size").and_then(Value::as_u64).unwrap_or(0).saturating_mul(1024);
            Ok(format!("size :{} {}", if obj.get("op").and_then(Value::as_str) == Some("smaller") { "under" } else { "over" }, bytes))
        }
        "date" => {
            requires.insert("date");
            requires.insert("relational");
            let op = obj.get("op").and_then(Value::as_str).unwrap_or("on");
            let day = quote(obj.get("value").and_then(Value::as_str).unwrap_or(""));
            let test = match op {
                "before" => format!("date :value \"lt\" :originalzone \"Date\" \"date\" \"{day}\""),
                "after" => format!("date :value \"gt\" :originalzone \"Date\" \"date\" \"{day}\""),
                "on-or-before" => format!("date :value \"le\" :originalzone \"Date\" \"date\" \"{day}\""),
                "on-or-after" => format!("date :value \"ge\" :originalzone \"Date\" \"date\" \"{day}\""),
                "not-on" => format!("not date :is :originalzone \"Date\" \"date\" \"{day}\""),
                _ => format!("date :is :originalzone \"Date\" \"date\" \"{day}\""),
            };
            Ok(test)
        }
        _ => Err(format!("unsupported condition {field}")),
    }
}

fn actions_to_sieve(actions: &[Value], requires: &mut BTreeSet<&'static str>) -> Result<String, String> {
    let mut flags = String::new();
    let mut redirects = String::new();
    let mut delivery = String::new();
    let mut discard = false;

    // Flags must be set before fileinto/keep so they apply to the delivered
    // copy regardless of the action order chosen in the UI.
    for action in actions {
        let obj = action.as_object().ok_or("action must be an object")?;
        let kind = obj.get("kind").and_then(Value::as_str).unwrap_or("");
        match kind {
            "label" => {
                requires.insert("imap4flags");
                let keyword = keyword_for_label(obj.get("value").and_then(Value::as_str).unwrap_or("Mail"));
                flags.push_str(&format!("    addflag \"{}\";\n", quote(&keyword)));
            }
            "mark-read" => {
                requires.insert("imap4flags");
                flags.push_str("    addflag \"\\\\Seen\";\n");
            }
            "mark-starred" => {
                requires.insert("imap4flags");
                flags.push_str("    addflag \"\\\\Flagged\";\n");
            }
            "forward" => {
                requires.insert("copy");
                let target = quote(obj.get("value").and_then(Value::as_str).unwrap_or(""));
                redirects.push_str(&format!("    redirect :copy \"{target}\";\n"));
            }
            "move" => {
                requires.insert("fileinto");
                delivery.push_str(&format!(
                    "    fileinto \"{}\";\n",
                    quote(obj.get("value").and_then(Value::as_str).unwrap_or("Inbox")),
                ));
            }
            "archive" => {
                requires.insert("fileinto");
                delivery.push_str("    fileinto \"Archive\";\n");
            }
            "keep-in-inbox" => delivery.push_str("    keep;\n"),
            "discard" => discard = true,
            _ => return Err(format!("unsupported action {kind}")),
        }
    }
    if discard {
        delivery.push_str("    discard;\n    stop;\n");
    }
    Ok(format!("{flags}{redirects}{delivery}"))
}

fn compile_script(
    rules: &[RuleRow],
    forwarding: Option<&ForwardRow>,
    vacation: Option<&VacationRow>,
    contacts: &[String],
) -> Result<(String, bool), String> {
    if rules.len() > MAX_RULES { return Err(format!("A maximum of {MAX_RULES} rules is supported")); }
    let mut requires = BTreeSet::new();
    let mut body = String::from("# Managed by CS Mail. Manual edits are overwritten.\n\n");
    let mut has_effect = false;


    if let Some(forward) = forwarding.filter(|f| f.enabled && f.verified_at.is_some() && valid_email(&f.target_email)) {
        requires.insert("copy");
        has_effect = true;
        if forward.keep_copy {
            body.push_str(&format!("redirect :copy \"{}\";\n\n", quote(&forward.target_email)));
        } else {
            body.push_str(&format!("redirect \"{}\";\nstop;\n\n", quote(&forward.target_email)));
        }
    }

    for rule in rules.iter().filter(|rule| rule.enabled) {
        validate_rule(&rule.name, &rule.conditions.0, &rule.actions.0)?;
        let conditions = rule.conditions.0.as_array().unwrap();
        let actions = rule.actions.0.as_array().unwrap();
        let mut tests = Vec::with_capacity(conditions.len());
        for condition in conditions { tests.push(condition_to_sieve(condition, &mut requires)?); }
        let test = if tests.len() == 1 { tests[0].clone() } else { format!("allof({})", tests.join(", ")) };
        let actions = actions_to_sieve(actions, &mut requires)?;
        body.push_str(&format!("# rule {} ({})\nif {} {{\n{}}}\n\n", rule.position + 1, quote(&rule.name), test, actions));
        has_effect = true;
    }

    if let Some(v) = vacation.filter(|v| v.enabled) {
        requires.insert("vacation");
        has_effect = true;
        let mut gates = Vec::<String>::new();
        if let Some(start) = v.starts_at {
            requires.insert("date"); requires.insert("relational");
            gates.push(format!("currentdate :value \"ge\" \"date\" \"{}\"", start.format("%Y-%m-%d")));
        }
        if let Some(end) = v.ends_at {
            requires.insert("date"); requires.insert("relational");
            gates.push(format!("currentdate :value \"le\" \"date\" \"{}\"", end.format("%Y-%m-%d")));
        }
        if v.only_contacts {
            let safe = contacts.iter().filter(|email| valid_email(email)).take(1000).map(|email| format!("\"{}\"", quote(email))).collect::<Vec<_>>();
            if safe.is_empty() {
                // Explicitly match nothing rather than accidentally replying to everyone.
                gates.push("false".into());
            } else {
                gates.push(format!("address :is \"From\" [{}]", safe.join(", ")));
            }
        }
        let action = format!(
            "vacation :days 1 :subject \"{}\" {};",
            quote(&v.subject),
            multiline_string(&v.message),
        );
        if gates.is_empty() { body.push_str(&format!("{action}\n")); }
        else if gates.len() == 1 { body.push_str(&format!("if {} {{\n    {}\n}}\n", gates[0], action)); }
        else { body.push_str(&format!("if allof({}) {{\n    {}\n}}\n", gates.join(", "), action)); }
    }

    let mut script = String::new();
    if !requires.is_empty() {
        let list = requires.iter().map(|item| format!("\"{item}\"")).collect::<Vec<_>>().join(", ");
        script.push_str(&format!("require [{list}];\n\n"));
    }
    script.push_str(&body);
    Ok((script, has_effect))
}

async fn script_inventory(
    state: &AppState,
    account: &str,
) -> Result<(Option<(String, bool)>, bool, Option<String>), String> {
    let result = state
        .stalwart
        .sieve_read("SieveScript/get", json!({ "accountId": account }))
        .await
        .map_err(provider_error)?;
    let state_id = result.get("state").and_then(Value::as_str).map(str::to_string);
    let list = result
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut managed = None;
    let mut foreign_active = false;
    for item in list {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        let active = item.get("isActive").and_then(Value::as_bool).unwrap_or(false);
        if (name == MANAGED_SCRIPT_NAME || name == LEGACY_MANAGED_SCRIPT_NAME) && !id.is_empty() {
            managed = Some((id.to_string(), active));
        } else if active {
            foreign_active = true;
        }
    }
    Ok((managed, foreign_active, state_id))
}

pub async fn sync_user(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<SyncOutcome, String> {
    entitlements::require_feature(state, user_id, "mail")
        .await
        .map_err(|e| e.to_string())?;

    // Session advisory lock serializes reconciliation for this mailbox across
    // concurrent requests and across multiple API instances.
    let mut lock = state.db.acquire().await.map_err(|e| e.to_string())?;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(mailbox_id.to_string())
        .execute(&mut *lock)
        .await
        .map_err(|e| e.to_string())?;

    let result = sync_user_locked(state, user_id, mailbox_id).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(mailbox_id.to_string())
        .execute(&mut *lock)
        .await;
    result
}

async fn sync_user_locked(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<SyncOutcome, String> {
    let revision: i64 = sqlx::query_scalar(
        "INSERT INTO mail_automation_state (user_id, mailbox_id, status) VALUES ($1, $2, 'pending')
         ON CONFLICT (mailbox_id) WHERE mailbox_id IS NOT NULL DO UPDATE SET status='syncing', updated_at=now()
         RETURNING desired_revision",
    )
    .bind(user_id)
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let result = sync_user_inner(state, user_id, mailbox_id, revision).await;
    match result {
        Ok(outcome) => {
            sqlx::query(
                "UPDATE mail_automation_state SET applied_revision=$2, provider_script_id=$3,
                    status=CASE WHEN desired_revision=$2 THEN $4 ELSE 'pending' END,
                    last_error='', retry_count=0, next_retry_at=now(), applied_at=now(), updated_at=now()
                 WHERE mailbox_id=$1",
            )
            .bind(mailbox_id)
            .bind(revision)
            .bind(&outcome.script_id)
            .bind(outcome.status)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
            Ok(outcome)
        }
        Err(error) => {
            let safe = error.chars().take(1000).collect::<String>();
            let _ = sqlx::query(
                "UPDATE mail_automation_state SET status='error', last_error=$2,
                    retry_count=retry_count+1,
                    next_retry_at=now() + (LEAST(3600, 5 * power(2, LEAST(retry_count, 9))::int) * interval '1 second'),
                    updated_at=now() WHERE mailbox_id=$1",
            )
            .bind(mailbox_id)
            .bind(&safe)
            .execute(&state.db)
            .await;
            Err(error)
        }
    }
}

async fn sync_user_inner(state: &AppState, user_id: Uuid, mailbox_id: Uuid, revision: i64) -> Result<SyncOutcome, String> {
    let (_email, account) = account_for(state, user_id, mailbox_id).await?;
    let (rules, forwarding, vacation, contacts) = desired_state(state, mailbox_id).await?;

    // Ensure the well-known Archive mailbox exists before a Sieve fileinto can target it.
    let needs_archive = rules.iter().filter(|r| r.enabled).any(|r| {
        r.actions.0.as_array().is_some_and(|items| items.iter().any(|item| item.get("kind").and_then(Value::as_str) == Some("archive")))
    });
    if needs_archive {
        imap::ensure_role_mailbox(&state.stalwart, &account, "archive", "Archive").await?;
    }

    let (script, has_effect) = compile_script(&rules, forwarding.as_ref(), vacation.as_ref(), &contacts)?;
    let (current, foreign_active, sieve_state) = script_inventory(state, &account).await?;
    if has_effect && foreign_active && !current.as_ref().is_some_and(|(_, active)| *active) {
        return Err("Another server-side mail filter is active. Disable it before enabling CS Mail automation".into());
    }

    if !has_effect {
        if let Some((id, true)) = current.as_ref() {
            state.stalwart.sieve_write("SieveScript/set", json!({
                "accountId": account,
                "ifInState": sieve_state,
                "onSuccessDeactivateScript": true
            })).await.map_err(provider_error)?;
            return Ok(SyncOutcome { status: "disabled", revision, script_id: Some(id.clone()) });
        }
        return Ok(SyncOutcome {
            status: "disabled",
            revision,
            script_id: current.as_ref().map(|(id, _)| id.clone()),
        });
    }

    let blob_id = state.stalwart.upload_sieve(&account, &script).await.map_err(provider_error)?;
    let validation = state.stalwart.sieve_read("SieveScript/validate", json!({
        "accountId": account,
        "blobId": blob_id
    })).await.map_err(provider_error)?;
    if let Some(error) = validation.get("error").filter(|value| !value.is_null()) {
        return Err(format!("generated Sieve script was rejected: {}", error));
    }

    let script_id = if let Some((id, _)) = current {
        let mut update = serde_json::Map::new();
        update.insert(id.clone(), json!({ "name": MANAGED_SCRIPT_NAME, "blobId": blob_id }));
        let response = state.stalwart.sieve_write("SieveScript/set", json!({
            "accountId": account,
            "ifInState": sieve_state,
            "update": Value::Object(update),
            "onSuccessActivateScript": id.clone()
        })).await.map_err(provider_error)?;
        if let Some(problem) = response.get("notUpdated").and_then(|value| value.get(&id)) {
            return Err(format!("managed Sieve update was rejected: {problem}"));
        }
        id
    } else {
        let response = state.stalwart.sieve_write("SieveScript/set", json!({
            "accountId": account,
            "ifInState": sieve_state,
            "create": { "cs": { "name": MANAGED_SCRIPT_NAME, "blobId": blob_id } },
            "onSuccessActivateScript": "#cs"
        })).await.map_err(provider_error)?;
        if let Some(problem) = response.get("notCreated").and_then(|value| value.get("cs")) {
            return Err(format!("managed Sieve create was rejected: {problem}"));
        }
        response.get("created").and_then(|value| value.get("cs")).and_then(|value| value.get("id")).and_then(Value::as_str)
            .ok_or_else(|| "mail provider did not return the managed script id".to_string())?.to_string()
    };
    Ok(SyncOutcome { status: "ready", revision, script_id: Some(script_id) })
}

pub async fn sync_status(state: &AppState, mailbox_id: Uuid) -> Result<Value, String> {
    let row: Option<(i64, i64, String, String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT desired_revision, applied_revision, status, last_error, applied_at
         FROM mail_automation_state WHERE mailbox_id=$1",
    ).bind(mailbox_id).fetch_optional(&state.db).await.map_err(|e| e.to_string())?;
    Ok(match row {
        Some((desired, applied, status, error, applied_at)) => json!({
            "status": status, "desiredRevision": desired, "appliedRevision": applied,
            "inSync": desired == applied && matches!(status.as_str(), "ready" | "disabled"),
            "lastError": error, "appliedAt": applied_at
        }),
        None => json!({ "status": "disabled", "desiredRevision": 0, "appliedRevision": 0, "inSync": true, "lastError": "", "appliedAt": null }),
    })
}

/// Return how many saved rules move messages into a named custom folder.
/// Disabled rules are included so a later re-enable cannot point at a folder
/// that no longer exists.
pub async fn folder_reference_count(
    state: &AppState,
    mailbox_id: Uuid,
    folder_name: &str,
) -> Result<i64, String> {
    sqlx::query_scalar(
        "SELECT count(*) FROM mail_rules r
         WHERE r.mailbox_id=$1 AND EXISTS (
           SELECT 1 FROM jsonb_array_elements(r.actions) action
           WHERE action->>'kind'='move' AND lower(action->>'value')=lower($2)
         )",
    )
    .bind(mailbox_id)
    .bind(folder_name)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())
}

/// Contact membership is compiled into the managed Sieve source when a
/// contacts-only vacation responder is enabled. Mark it dirty on contact CRUD
/// so the provider copy cannot silently drift.
pub async fn contacts_changed(state: &AppState, user_id: Uuid, mailbox_id: Uuid) -> Result<(), String> {
    let depends_on_contacts: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM mail_vacation
                       WHERE mailbox_id=$1 AND enabled AND only_contacts)",
    )
    .bind(mailbox_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    if !depends_on_contacts {
        return Ok(());
    }
    mark_dirty(state, user_id, mailbox_id).await?;
    let worker_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = sync_user(&worker_state, user_id, mailbox_id).await {
            tracing::warn!(user_id=%user_id, %error, "contacts changed; vacation sync queued for reconciliation");
        }
    });
    Ok(())
}

pub fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let ids: Vec<(Uuid, Uuid)> = match sqlx::query_as(
                "SELECT user_id, mailbox_id FROM mail_automation_state
                 WHERE mailbox_id IS NOT NULL AND status IN ('pending','error') AND next_retry_at <= now()
                 ORDER BY next_retry_at LIMIT 25",
            ).fetch_all(&state.db).await {
                Ok(ids) => ids,
                Err(error) => { tracing::warn!(%error, "mail automation retry scan failed"); continue; }
            };
            for (user_id, mailbox_id) in ids {
                if let Err(error) = sync_user(&state, user_id, mailbox_id).await {
                    tracing::warn!(user_id=%user_id, %error, "mail automation reconciliation failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_rule_parts() {
        assert!(validate_rule("x", &json!([{"field":"wat"}]), &json!([{"kind":"archive"}])).is_err());
        assert!(validate_rule("x", &json!([{"field":"from","value":"a"}]), &json!([{"kind":"wat"}])).is_err());
    }

    #[test]
    fn compiles_safe_sieve_literals_and_actions() {
        let rule = RuleRow {
            id: Uuid::new_v4(), name: "Boss \"urgent\"".into(), enabled: true, position: 0,
            conditions: Json(json!([{"field":"from","value":"boss@example.com"}])),
            actions: Json(json!([{"kind":"mark-starred"},{"kind":"archive"}])),
        };
        let (script, active) = compile_script(&[rule], None, None, &[]).unwrap();
        assert!(active);
        assert!(script.contains("imap4flags"));
        assert!(script.contains("fileinto \"Archive\""));
        assert!(script.contains("addflag \"\\\\Flagged\""));
    }

    #[test]
    fn vacation_contacts_fail_closed_when_contact_list_empty() {
        let vacation = VacationRow { enabled: true, subject: "Away".into(), message: "Back soon".into(), only_contacts: true, starts_at: None, ends_at: None };
        let (script, active) = compile_script(&[], None, Some(&vacation), &[]).unwrap();
        assert!(active);
        assert!(script.contains("if false"));
    }
}
