//! End-to-end API flows against a real Postgres (WS8.5): auth, contacts,
//! calendar, role gating and the metrics scrape. The suite is a no-op unless
//! `TEST_DATABASE_URL` (or `DATABASE_URL`) points at a disposable database, so
//! `cargo test` stays green for contributors without a DB. CI provides one.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use cs_mail_api::metrics::Metrics;
use cs_mail_api::middleware::rate_limit::RateLimiter;
use cs_mail_api::router::build_router;
use cs_mail_api::services::provisioning::ProvisioningService;
use cs_mail_api::services::smtp::SmtpConfig;
use cs_mail_api::services::stalwart::{StalwartConfig, StalwartService};
use cs_mail_api::state::AppState;
use cs_mail_api::ws::EventHub;

struct TestApp {
    app: Router,
    db: PgPool,
}

const TEST_PROVISIONING_KEY: &str = "integration-provisioning-key-0123456789";

/// Build the app against a throwaway database, or `None` to skip the suite.
async fn test_app() -> Option<TestApp> {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to TEST_DATABASE_URL");
    sqlx::migrate!().run(&db).await.expect("run migrations");
    // Integration flows exercise the customer lifecycle in an isolated DB.
    // Production keeps these controls closed until the operator opens them.
    sqlx::query("UPDATE platform_controls SET public_signup_enabled=TRUE, business_creation_enabled=TRUE, plan_ordering_enabled=TRUE, domain_onboarding_enabled=TRUE, mailbox_provisioning_enabled=TRUE, outbound_sending_enabled=TRUE WHERE singleton=TRUE")
        .execute(&db).await.expect("enable isolated test lifecycle");

    let state = AppState {
        db: db.clone(),
        environment: "test".into(),
        release_sha256: "test".into(),
        jwt_secret: "integration-test-secret".into(),
        jwt_access_ttl_secs: 900,
        jwt_refresh_ttl_secs: 2_592_000,
        delivery_event_secret: None,
        cors_origins: vec![],
        hub: EventHub::new(),
        realtime_instance_id: Uuid::new_v4(),
        realtime_poll_secs: 2,
        realtime_lease_secs: 60,
        realtime_batch_size: 20,
        realtime_event_retention_secs: 604_800,
        public_origin: "http://localhost:5174".into(),
        // No verification gate and no Stalwart bridge: flows stay hermetic.
        require_verification: false,
        return_token_links: true,
        cookie_secure: false,
        stalwart: StalwartService::new(StalwartConfig {
            admin_url: String::new(),
            admin_username: String::new(),
            admin_secret: String::new(),
            admin_bearer_token: None,
            mail_jmap_username: String::new(),
            mail_jmap_secret: String::new(),
            default_domain: "example.test".into(),
            ownership_namespace: "cs-mail".into(),
            request_timeout: std::time::Duration::from_secs(1),
            read_retries: 0,
            retry_base_delay: std::time::Duration::from_millis(1),
            smtp: SmtpConfig::default(),
        })
        .expect("build disabled test mail provider"),
        system_mailer: None,
        provisioning: ProvisioningService::new(
            TEST_PROVISIONING_KEY.into(),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(60),
            3,
            5,
        )
        .expect("build provisioning service"),
        two_factor_key: "integration-two-factor-key-0123456789".into(),
        mail_client_host: "mail.example.test".into(),
        mail_client_imap_port: 993,
        mail_client_smtp_port: 587,
        mail_client_max_app_passwords: 5,
        mail_import_max_bytes: 64 * 1024 * 1024,
        mail_import_message_max_bytes: 16 * 1024 * 1024,
        mail_import_poll_secs: 1,
        mail_import_lease_secs: 60,
        schedule_poll_secs: 1,
        schedule_lease_secs: 60,
        schedule_retry_base_secs: 1,
        schedule_max_attempts: 3,
        schedule_batch_size: 10,
        attachment_store_dir: std::env::temp_dir().join(format!(
            "cs-mail-api-flows-{}",
            Uuid::new_v4()
        )),
        attachment_staging_quota_bytes: 1024 * 1024 * 1024,
        attachment_upload_ttl_secs: 86_400,
        attachment_draft_ttl_secs: 2_592_000,
        attachment_consumed_grace_secs: 3_600,
        attachment_cleanup_secs: 900,
        billing_instant_activation: true,
        rate: RateLimiter::new(db.clone(), "integration-test-rate-limit-key-material-32bytes", vec!["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]),
        metrics: Arc::new(Metrics::new()),
    };

    Some(TestApp {
        app: build_router(state),
        db,
    })
}

fn unique_email() -> String {
    format!("it-{}@example.test", Uuid::new_v4().simple())
}

/// Build a request with a JSON body and a synthetic peer address (handlers
/// extract `ConnectInfo`).
fn req(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = body.map_or(Body::empty(), |v| Body::from(v.to_string()));
    let mut request = builder.body(body).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    request
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// Send a request and return (status, Content-Type header, raw body bytes) without
/// JSON parsing — used for CSV-download endpoints whose bodies are not JSON.
async fn send_raw(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes: Vec<u8> = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, content_type, bytes)
}

/// Send a request and return (status, Set-Cookie header, body).
async fn send_headers(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, set_cookie, value)
}

fn session_cookie(set_cookie: &str) -> String {
    let first = set_cookie.split(';').next().unwrap_or("");
    first
        .strip_prefix("cs_mail_session=")
        .expect("cs_mail_session cookie")
        .to_string()
}

fn with_cookie(request: &mut Request<Body>, cookie_value: &str) {
    let value = format!("cs_mail_session={cookie_value}");
    request
        .headers_mut()
        .insert(header::COOKIE, value.parse::<axum::http::HeaderValue>().unwrap());
}

/// Register a member and return `(access_token, user_id, email)`.
async fn register(app: &Router) -> (String, String, String) {
    let email = unique_email();
    let (status, body) = send(
        app,
        req(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "name": "Integration", "email": email.clone(), "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register failed: {body}");
    let token = body["access"].as_str().expect("access token").to_string();
    let id = body["user"]["id"].as_str().expect("user id").to_string();
    (token, id, email)
}

/// Explicitly attach a hosted mailbox for integration flows that exercise mail
/// features. Upgrade 19 intentionally makes `register()` platform-only, so
/// tests must opt into mailbox authority instead of relying on login email.
async fn attach_test_mailbox(db: &PgPool, user_id: &str, email: &str) -> Uuid {
    let user_id = Uuid::parse_str(user_id).expect("test user id");
    let organization_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE is_system=TRUE LIMIT 1",
    )
    .fetch_one(db)
    .await
    .expect("system organization");

    sqlx::query(
        "INSERT INTO organization_memberships(organization_id,user_id,role,status)
         VALUES($1,$2,'member','active')
         ON CONFLICT(organization_id,user_id) DO UPDATE
         SET status='active',updated_at=now()",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(db)
    .await
    .expect("test mailbox membership");

    sqlx::query(
        "INSERT INTO organization_domains(organization_id,domain,status,is_system,activated_at)
         VALUES($1,'example.test','active',TRUE,now())
         ON CONFLICT DO NOTHING",
    )
    .bind(organization_id)
    .execute(db)
    .await
    .expect("test mailbox domain");

    let domain_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM organization_domains WHERE lower(domain::text)='example.test'",
    )
    .fetch_one(db)
    .await
    .expect("test mailbox domain id");

    let local_part = email.split('@').next().expect("test local part");
    let mailbox_id: Uuid = sqlx::query_scalar(
        "INSERT INTO mailboxes(
           organization_id,domain_id,user_id,address,local_part,display_name,status,
           is_primary_for_user,sync_status,quota_bytes
         ) VALUES($1,$2,$3,$4,$5,'Integration','active',TRUE,'ready',5368709120)
         RETURNING id",
    )
    .bind(organization_id)
    .bind(domain_id)
    .bind(user_id)
    .bind(email)
    .bind(local_part)
    .fetch_one(db)
    .await
    .expect("test mailbox row");

    sqlx::query(
        "UPDATE users
         SET active_organization_id=$1,primary_mailbox_id=$2,mail_sync_status='ready',updated_at=now()
         WHERE id=$3",
    )
    .bind(organization_id)
    .bind(mailbox_id)
    .bind(user_id)
    .execute(db)
    .await
    .expect("attach primary test mailbox");

    mailbox_id
}

#[tokio::test]
async fn auth_contacts_and_calendar_flow() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, user_id, email) = register(&t.app).await;

    // Upgrade 19: public registration creates a platform identity only.
    // A login such as Gmail/Outlook must never be interpreted as a hosted
    // mailbox or queued for provider provisioning before business/domain setup.
    let (primary_mailbox_id, mail_sync_status): (Option<Uuid>, String) = sqlx::query_as(
        "SELECT primary_mailbox_id, mail_sync_status FROM users WHERE id=$1",
    )
    .bind(Uuid::parse_str(&user_id).unwrap())
    .fetch_one(&t.db)
    .await
    .expect("platform identity row");
    assert!(primary_mailbox_id.is_none());
    assert_eq!(mail_sync_status, "none");
    let provisioning_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provisioning_jobs WHERE user_id=$1",
    )
    .bind(Uuid::parse_str(&user_id).unwrap())
    .fetch_one(&t.db)
    .await
    .expect("platform registration provisioning count");
    assert_eq!(provisioning_jobs, 0);

    // Duplicate signup is rejected.
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "name": "Dup", "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Wrong password then correct password.
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "wrong-password" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, login) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(login["user"]["id"].as_str(), Some(user_id.as_str()));

    // Protected endpoint requires a bearer token.
    let (status, _) = send(&t.app, req("GET", "/api/contacts", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A new login has no business, plan, or mailbox yet. Profile remains
    // readable so onboarding can begin without inventing entitlements.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK, "profile: {profile}");
    assert!(profile["plan"].is_null());
    assert_eq!(profile["has_mailbox"], false);
    assert_eq!(profile["storage"]["total_bytes"], 0);

    // Contacts: create -> list -> update -> delete.
    let (status, created) = send(
        &t.app,
        req(
            "POST",
            "/api/contacts",
            Some(&access),
            Some(json!({ "name": "Ada", "email": "ada@example.test", "company": "Analytical" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "contact create: {created}");
    let contact_id = created["id"].as_str().expect("contact id").to_string();
    let contact_version = created["version"].as_i64().expect("contact version");

    let (status, list) = send(&t.app, req("GET", "/api/contacts", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["contacts"].as_array().unwrap().len(), 1);

    let (status, updated) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/contacts/{contact_id}"),
            Some(&access),
            Some(json!({ "phone": "+1-555-0100", "version": contact_version })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "contact update: {updated}");
    assert_eq!(updated["phone"], "+1-555-0100");
    let updated_contact_version = updated["version"].as_i64().expect("updated contact version");

    let (status, _) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/contacts/{contact_id}"),
            Some(&access),
            Some(json!({ "phone": "+1-555-0199", "version": contact_version })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "stale contact versions must not overwrite newer state");

    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/contacts/{contact_id}?version={updated_contact_version}"),
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Calendar: create -> list -> delete.
    let (status, event) = send(
        &t.app,
        req(
            "POST",
            "/api/calendar/events",
            Some(&access),
            Some(json!({
                "title": "Standup",
                "date": "2026-01-15",
                "start": "09:00",
                "end": "09:15",
                "category": "meeting",
                "timezoneOffsetMinutes": 180,
                "invitees": [
                    {"email": "person@example.test", "status": "pending"},
                    {"email": "PERSON@example.test", "status": "pending"}
                ],
                "recurrence": {"frequency": "weekly", "interval": 1, "count": 2}
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "calendar create: {event}");
    let event_id = event["id"].as_str().expect("event id").to_string();
    let event_version = event["version"].as_i64().expect("event version");
    assert_eq!(event["invitees"].as_array().unwrap().len(), 1, "attendee addresses dedupe case-insensitively");

    let (status, events) = send(
        &t.app,
        req("GET", "/api/calendar/events", Some(&access), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(events["events"].as_array().is_some_and(|a| !a.is_empty()));

    let (status, recurring) = send(
        &t.app,
        req(
            "GET",
            "/api/calendar/events?start=2026-01-15&end=2026-01-22&timezoneOffsetMinutes=180",
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "recurring range: {recurring}");
    assert_eq!(recurring["events"].as_array().unwrap().len(), 2);
    assert_eq!(recurring["events"][1]["seriesId"].as_str(), Some(event_id.as_str()));

    let update_body = json!({
        "title": "Standup updated",
        "date": "2026-01-15",
        "start": "09:00",
        "end": "09:15",
        "category": "meeting",
        "timezoneOffsetMinutes": 180,
        "invitees": [{"email": "person@example.test", "status": "accepted"}],
        "recurrence": {"frequency": "weekly", "interval": 1, "count": 2},
        "version": event_version
    });
    let (status, changed) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/calendar/events/{event_id}"),
            Some(&access),
            Some(update_body.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "calendar update: {changed}");
    assert_eq!(changed["title"], "Standup updated");
    let changed_version = changed["version"].as_i64().expect("updated event version");

    let (status, _) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/calendar/events/{event_id}"),
            Some(&access),
            Some(update_body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "stale event versions must not overwrite newer state");

    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/calendar/events/{event_id}?version={changed_version}"),
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn scheduled_creation_is_idempotent_and_dead_letters_redrive() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, user_id, email) = register(&t.app).await;
    attach_test_mailbox(&t.db, &user_id, &email).await;
    let send_key = Uuid::new_v4();
    let idem = format!("schedule:{send_key}");
    let body = json!({
        "send_at": "2032-01-15T09:00:00Z",
        "to": [{ "name": "Receiver", "email": "receiver@example.test" }],
        "cc": [],
        "bcc": [],
        "subject": "Idempotent schedule",
        "body_text": "Only one row should exist",
        "attachments": [],
        "send_key": send_key,
    });

    let mut first = req("POST", "/api/scheduled", Some(&access), Some(body.clone()));
    first
        .headers_mut()
        .insert("idempotency-key", idem.parse().unwrap());
    let (status, created) = send(&t.app, first).await;
    assert_eq!(status, StatusCode::CREATED, "schedule create: {created}");
    let id = created["id"].as_str().expect("scheduled id").to_string();

    let mut second = req("POST", "/api/scheduled", Some(&access), Some(body.clone()));
    second
        .headers_mut()
        .insert("idempotency-key", idem.parse().unwrap());
    let (status, duplicate) = send(&t.app, second).await;
    assert_eq!(status, StatusCode::OK, "schedule retry: {duplicate}");
    assert_eq!(duplicate["id"].as_str(), Some(id.as_str()));

    let mut changed = body;
    changed["subject"] = json!("Different content");
    let mut third = req("POST", "/api/scheduled", Some(&access), Some(changed));
    third
        .headers_mut()
        .insert("idempotency-key", idem.parse().unwrap());
    let (status, _) = send(&t.app, third).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scheduled_sends WHERE idempotency_key = $1",
    )
    .bind(&idem)
    .fetch_one(&t.db)
    .await
    .expect("scheduled row count");
    assert_eq!(count, 1);

    sqlx::query(
        "UPDATE scheduled_sends
         SET status = 'dead', attempt_count = 12, error = 'temporary provider failure',
             completed_at = now(), next_attempt_at = NULL
         WHERE id = $1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&t.db)
    .await
    .expect("dead-letter row");

    let (status, redrive) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/scheduled/{id}/retry"),
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "schedule redrive: {redrive}");
    assert_eq!(redrive["status"], "retry");

    let (status, cancelled) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/scheduled/{id}"),
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "schedule cancel: {cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
}

#[tokio::test]
async fn organization_membership_isolation() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (owner_token, owner_id, _owner_email) = register(&t.app).await;
    let (other_token, other_id, _other_email) = register(&t.app).await;

    let (status, created) = send(
        &t.app,
        req(
            "POST",
            "/api/organizations",
            Some(&owner_token),
            Some(json!({ "name": "Isolation Test Business" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create business: {created}");
    let organization_id = created["id"].as_str().expect("organization id");
    let subscription_status: String = sqlx::query_scalar(
        "SELECT status FROM organization_subscriptions WHERE organization_id=$1",
    )
    .bind(Uuid::parse_str(organization_id).unwrap())
    .fetch_one(&t.db)
    .await
    .expect("business subscription status");
    assert_eq!(subscription_status, "suspended", "new businesses must not receive a plan before ordering");
    let (status, _) = send(&t.app, req("POST", &format!("/api/organizations/{organization_id}/domains"), Some(&owner_token), Some(json!({"domain":"example-business.test"})))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "domain setup requires an active plan");

    let owner_membership: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_memberships WHERE organization_id=$1 AND user_id=$2",
    )
    .bind(Uuid::parse_str(organization_id).unwrap())
    .bind(Uuid::parse_str(&owner_id).unwrap())
    .fetch_optional(&t.db)
    .await
    .expect("owner membership query");
    assert_eq!(owner_membership.as_deref(), Some("owner"));

    // Guessing a valid organization UUID never grants tenant access.
    let (status, _) = send(
        &t.app,
        req(
            "GET",
            &format!("/api/organizations/{organization_id}"),
            Some(&other_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &t.app,
        req(
            "GET",
            &format!("/api/organizations/{organization_id}/members"),
            Some(&other_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/organizations/{organization_id}/activate"),
            Some(&other_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Neither public registration was silently enrolled into the protected
    // CrescentSphere system organization.
    for user_id in [owner_id, other_id] {
        let protected_memberships: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM organization_memberships om
             JOIN organizations o ON o.id=om.organization_id
             WHERE om.user_id=$1 AND o.is_system=TRUE",
        )
        .bind(Uuid::parse_str(&user_id).unwrap())
        .fetch_one(&t.db)
        .await
        .expect("protected membership count");
        assert_eq!(protected_memberships, 0);
    }
}

#[tokio::test]
async fn role_gate_and_metrics_scrape() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (member_token, _id, email) = register(&t.app).await;

    // A member is forbidden from the admin surface.
    let (status, _) = send(
        &t.app,
        req("GET", "/api/admin/users", Some(&member_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Promote server-side, then a fresh login carries the admin role.
    sqlx::query("UPDATE users SET role = 'admin', platform_role='platform_admin' WHERE email = $1")
        .bind(&email)
        .execute(&t.db)
        .await
        .expect("promote to admin");

    let (status, login) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(login["user"]["role"], "admin");
    let admin_token = login["access"].as_str().unwrap().to_string();

    let (status, users) = send(
        &t.app,
        req("GET", "/api/admin/users", Some(&admin_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin users: {users}");
    assert!(users["users"].as_array().is_some());

    // Metrics scrape is public and exposes the CS Mail series.
    let response = t
        .app
        .clone()
        .oneshot(req("GET", "/api/metrics", None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("cs_mail_http_requests_total"));
    assert!(text.contains("cs_mail_db_pool_connections"));
}

#[tokio::test]
async fn admin_provisions_and_resets_user() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (member_token, _id, admin_email) = register(&t.app).await;
    sqlx::query("UPDATE users SET role = 'admin', platform_role='platform_admin' WHERE email = $1")
        .bind(&admin_email)
        .execute(&t.db)
        .await
        .expect("promote to admin");

    let login = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": admin_email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(login.0, StatusCode::OK);
    let admin_token = login.1["access"].as_str().unwrap().to_string();

    // A member is rejected from provisioning accounts.
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            "/api/admin/users",
            Some(&member_token),
            Some(json!({
                "email": "provisioned@example.test",
                "display_name": "Provisioned User",
                "password": "Str0ng-Pass!23"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin creates a verified user with an explicit role/plan/quota.
    let (status, created) = send(
        &t.app,
        req(
            "POST",
            "/api/admin/users",
            Some(&admin_token),
            Some(json!({
                "email": "provisioned@example.test",
                "display_name": "Provisioned User",
                "password": "Str0ng-Pass!23",
                "role": "admin",
                "plan": "solo",
                "quota_bytes": 4i64 * 1024 * 1024 * 1024
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin create user: {created}");
    let user_id = created["id"].as_str().unwrap().to_string();

    // Appears in the list with the admin-applied plan and quota.
    let (status, users) = send(
        &t.app,
        req("GET", "/api/admin/users", Some(&admin_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let row = users["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"].as_str() == Some(&user_id))
        .expect("provisioned user listed");
    assert_eq!(row["role"], "admin");
    assert_eq!(row["plan"], "solo");
    assert_eq!(row["quota_bytes"], 4i64 * 1024 * 1024 * 1024);
    assert_eq!(row["quota_source"], "override");

    // Resetting the explicit exception returns the mailbox to the authoritative
    // Solo plan quota instead of preserving a hidden per-user number.
    let (status, reset_quota) = send(
        &t.app,
        req(
            "PATCH",
            &format!("/api/admin/users/{user_id}"),
            Some(&admin_token),
            Some(json!({ "reset_quota_override": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reset plan quota: {reset_quota}");
    assert_eq!(reset_quota["quota_source"], "plan");
    assert_eq!(reset_quota["quota_bytes"], 5i64 * 1024 * 1024 * 1024);

    // Admin resets the password; the new one signs in immediately.
    let (status, patched) = send(
        &t.app,
        req(
            "PATCH",
            &format!("/api/admin/users/{user_id}"),
            Some(&admin_token),
            Some(json!({ "password": "Rotated-Pass!99" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "password reset: {patched}");
    let (status, relogin) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": "provisioned@example.test", "password": "Rotated-Pass!99" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "relogin with rotated password: {relogin}"
    );

    // Admin erases the provisioned account; the audit trail records it.
    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/admin/users/{user_id}"),
            Some(&admin_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, audit) = send(
        &t.app,
        req("GET", "/api/admin/audit", Some(&admin_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let actions: Vec<&str> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(actions.contains(&"admin.user.created"));
    assert!(actions.contains(&"admin.user.password_reset"));
    assert!(actions.contains(&"admin.user.erase"));

    // Admin exports the audit trail as an RFC 4180 CSV stream (WS5.5 / gate 3.4).
    let (status, content_type, csv) = send_raw(&t.app, req("GET", "/api/admin/audit/export", Some(&admin_token), None)).await;
    assert_eq!(status, StatusCode::OK, "audit CSV export status");
    assert_eq!(
        content_type.as_deref(),
        Some("text/csv; charset=utf-8"),
        "audit CSV content-type"
    );
    let csv_text = String::from_utf8(csv).unwrap();
    let lines: Vec<&str> = csv_text.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines[0],
        "id,time,actor,action,detail",
        "audit CSV header row (RFC 4180)"
    );
    assert!(lines.len() >= 4, "audit CSV should have header + >2 entries, got {}", lines.len());
    assert!(
        csv_text.contains(&"admin.user.erase"),
        "audit CSV must include the erase action"
    );
    assert!(
        csv_text.contains(&"admin.user.created"),
        "audit CSV must include the create action"
    );
}

#[tokio::test]
async fn billing_manual_payment_flow() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    // A verified login creates a billing identity without receiving a plan.
    let (token, _user_id, _email) = register(&t.app).await;
    let (status, created) = send(&t.app, req("POST", "/api/organizations", Some(&token), Some(json!({"name":"Billing Test Business"})))).await;
    assert_eq!(status, StatusCode::OK, "create billing business: {created}");
    let organization_id = Uuid::parse_str(created["id"].as_str().expect("billing organization id")).unwrap();
    let (status, billing) = send(&t.app, req("GET", "/api/billing", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK, "billing summary: {billing}");
    assert_eq!(billing["current_plan"]["code"], "solo");
    assert_eq!(billing["subscription_status"], "suspended");
    assert!(billing["settings"]["bank_details"].is_string());
    assert!(billing["orders"].as_array().unwrap().is_empty());

    // Pricing list exposes all three seeded plans.
    let (status, plans) = send(&t.app, req("GET", "/api/billing/plans", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plans["plans"].as_array().unwrap().len(), 3);

    // Place an order for Team. Upgrade 29 test mode activates it immediately, while the invoice remains due until manual payment is reviewed.
    let (status, order) = send(
        &t.app,
        req(
            "POST",
            "/api/billing/orders",
            Some(&token),
            Some(json!({ "plan_code": "team", "payment_method": "bank", "customer_note": "Upgrade" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create order: {order}");
    assert_eq!(order["status"], "pending");
    assert_eq!(order["amount_cents"], 15900);
    assert_eq!(order["invoice_status"], "issued");
    assert_eq!(order["activation_mode"], "test_instant");
    assert!(order["invoice_number"].as_str().is_some_and(|v| v.starts_with("INV-")));

    // The plan is already active before a payment reference is submitted.
    let (status, immediate_profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(immediate_profile["plan"], "team");
    assert_eq!(immediate_profile["storage"]["total_bytes"], 10i64 * 1024 * 1024 * 1024);

    let order_id = order["id"].as_str().unwrap().to_string();

    let (status, submitted) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/billing/orders/{order_id}/paid"),
            Some(&token),
            Some(json!({ "payment_method": "bank", "payment_reference": "TRX-2026-0001" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "submit paid: {submitted}");
    assert_eq!(submitted["status"], "submitted");
    assert_eq!(submitted["payment_reference"], "TRX-2026-0001");

    // A member cannot touch the admin payment queue.
    let (status, _) = send(&t.app, req("GET", "/api/admin/orders", Some(&token), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin reviews the payment. In instant-test mode this marks the invoice paid without re-applying the subscription.
    let (_member_token, _id, admin_email) = register(&t.app).await;
    sqlx::query("UPDATE users SET role = 'admin', platform_role='platform_admin' WHERE email = $1")
        .bind(&admin_email)
        .execute(&t.db)
        .await
        .expect("promote billing reviewer");

    let (status, login) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": admin_email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin_token = login["access"].as_str().unwrap().to_string();

    let (status, queue) = send(
        &t.app,
        req("GET", "/api/admin/orders", Some(&admin_token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin queue: {queue}");
    assert!(queue["orders"].as_array().is_some_and(|a| a
        .iter()
        .any(|o| o["id"].as_str() == Some(order_id.as_str()))));

    let (status, approved) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/admin/orders/{order_id}/approve"),
            Some(&admin_token),
            Some(json!({ "admin_note": "Received via bank" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve: {approved}");
    assert_eq!(approved["status"], "paid");
    assert!(approved["invoice_number"]
        .as_str()
        .unwrap()
        .starts_with("INV-"));

    // The customer remains on Team, and the invoice appears in their list.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["plan"], "team");
    assert_eq!(profile["storage"]["total_bytes"], 10i64 * 1024 * 1024 * 1024);
    assert_eq!(profile["entitlements"]["quota_source"], "plan");

    let (status, invoices) = send(
        &t.app,
        req("GET", "/api/billing/invoices", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invoices["invoices"].as_array().unwrap().len(), 1);

    // An upgrade invoice must not displace a paid plan before review, even
    // while initial-order test activation is enabled.
    let (status, order2) = send(
        &t.app,
        req(
            "POST",
            "/api/billing/orders",
            Some(&token),
            Some(
                json!({ "plan_code": "business", "payment_method": "bank", "customer_note": "" }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(order2["activation_mode"], "payment_approval");
    let order2_id = order2["id"].as_str().unwrap().to_string();
    let (status, before_reject_profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before_reject_profile["plan"], "team");
    let (status, rejected) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/admin/orders/{order2_id}/reject"),
            Some(&admin_token),
            Some(json!({ "admin_note": "No payment received" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["invoice_status"], "void");
    let (status, after_reject_profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_reject_profile["plan"], "team", "rejected upgrade must leave the paid plan active");

    let (status, downgrade) = send(&t.app, req("POST", "/api/billing/orders", Some(&token),
        Some(json!({ "plan_code": "solo", "mailbox_count": 1, "payment_method": "bank", "customer_note": "" })))).await;
    assert_eq!(status, StatusCode::CONFLICT, "active-term downgrade: {downgrade}");

    // Customer withdraws an open invoice: a fresh Team order
    // is opened, then cancelled before the admin acts. The approved plan is NOT
    // touched — it stays ``team`` until the admin assigns another.
    let (status, order3) = send(
        &t.app,
        req(
            "POST",
            "/api/billing/orders",
            Some(&token),
            Some(json!({ "plan_code": "team", "payment_method": "bank", "customer_note": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "open order3: {order3}");
    assert_eq!(order3["status"], "pending");
    assert_eq!(order3["activation_mode"], "payment_approval");
    let order3_id = order3["id"].as_str().unwrap().to_string();

    let (status, cancelled) = send(
        &t.app,
        req(
            "POST",
            &format!("/api/billing/orders/{order3_id}/cancel"),
            Some(&token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "cancel order3: {cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["plan_code"], "team");

    // The test-activated plan survives invoice cancellation: the member is still on Team.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["plan"], "team", "cancelling an unpaid test invoice must not roll back the test plan");

    // Production lifecycle regression: period expiry must enter grace first,
    // then suspend after the grace deadline. Test-only instant activation must
    // never resurrect an expired/suspended paid estate.
    sqlx::query("UPDATE organization_subscriptions SET status='active',current_period_end=now()-interval '1 minute',renewal_grace_end=NULL WHERE organization_id=$1")
        .bind(organization_id).execute(&t.db).await.expect("expire subscription for lifecycle test");
    let (status, diag) = send(&t.app, req("GET", "/api/admin/diagnostics", Some(&admin_token), None)).await;
    assert_eq!(status, StatusCode::OK, "diagnostics lifecycle tick: {diag}");
    let lifecycle_status: String = sqlx::query_scalar("SELECT status FROM organization_subscriptions WHERE organization_id=$1")
        .bind(organization_id).fetch_one(&t.db).await.expect("past due status");
    assert_eq!(lifecycle_status, "past_due", "expiry must enter grace before suspension");
    let grace_end: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT renewal_grace_end FROM organization_subscriptions WHERE organization_id=$1")
        .bind(organization_id).fetch_one(&t.db).await.expect("grace deadline");
    assert!(grace_end.is_some(), "past-due lifecycle must have an explicit grace deadline");

    sqlx::query("UPDATE organization_subscriptions SET status='past_due',renewal_grace_end=now()-interval '1 minute' WHERE organization_id=$1")
        .bind(organization_id).execute(&t.db).await.expect("expire grace period");
    let (status, diag) = send(&t.app, req("GET", "/api/admin/diagnostics", Some(&admin_token), None)).await;
    assert_eq!(status, StatusCode::OK, "suspension lifecycle tick: {diag}");
    let lifecycle_status: String = sqlx::query_scalar("SELECT status FROM organization_subscriptions WHERE organization_id=$1")
        .bind(organization_id).fetch_one(&t.db).await.expect("suspended status");
    assert_eq!(lifecycle_status, "suspended", "expired grace must suspend the subscription");

    let (status, recovery_order) = send(&t.app, req("POST", "/api/billing/orders", Some(&token),
        Some(json!({ "plan_code": "team", "payment_method": "bank", "customer_note": "Recovery" })))).await;
    assert_eq!(status, StatusCode::CREATED, "suspended recovery order: {recovery_order}");
    assert_eq!(recovery_order["activation_mode"], "payment_approval", "test instant activation is bootstrap-only");
    let still_suspended: String = sqlx::query_scalar("SELECT status FROM organization_subscriptions WHERE organization_id=$1")
        .bind(organization_id).fetch_one(&t.db).await.expect("recovery pending status");
    assert_eq!(still_suspended, "suspended", "placing a recovery invoice must not reactivate service before payment approval");
    let recovery_order_id = recovery_order["id"].as_str().unwrap();
    let (status, _) = send(&t.app, req("POST", &format!("/api/billing/orders/{recovery_order_id}/cancel"), Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);

    // Issued invoices remain part of the audit trail even when rejected/cancelled; their invoice status becomes void.
    let (status, invoices) = send(
        &t.app,
        req("GET", "/api/billing/invoices", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        invoices["invoices"].as_array().unwrap().len(),
        4,
        "paid and void issued invoices remain visible"
    );

    // Admin plan CRUD: create, patch, deactivate. The code is unique per run
    // so the suite stays green against a database that keeps prior runs' rows.
    let reseller_code = format!("reseller-{}", Uuid::new_v4().simple());
    let (status, created) = send(
        &t.app,
        req(
            "POST",
            "/api/admin/plans",
            Some(&admin_token),
            Some(json!({
                "code": reseller_code, "name": "Reseller", "price_cents": 1500,
                "mailbox_bytes": 107374182400i64, "max_attachment_bytes": 10485760,
                "max_recipients": 200, "daily_send_limit": 5000, "seats": 10,
                "features": ["10 mailboxes"],
                "feature_flags": {
                    "mail": true, "attachments": true, "scheduled_send": true,
                    "read_receipts": true, "contacts": false, "calendar": true
                },
                "sort_order": 4, "active": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create plan: {created}");
    assert_eq!(created["plan"]["price"], "SAR 15.00");
    assert_eq!(created["plan"]["feature_flags"]["contacts"], false);

    let (status, updated) = send(
        &t.app,
        req(
            "PATCH",
            &format!("/api/admin/plans/{reseller_code}"),
            Some(&admin_token),
            Some(json!({
                "code": reseller_code, "name": "Reseller Plus", "price_cents": 2000,
                "mailbox_bytes": 107374182400i64, "max_attachment_bytes": 10485760,
                "max_recipients": 200, "daily_send_limit": 5000, "seats": 10,
                "features": [], "sort_order": 4, "active": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "patch plan: {updated}");
    assert_eq!(updated["plan"]["name"], "Reseller Plus");
    assert_eq!(updated["plan"]["feature_flags"]["contacts"], false);

    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/admin/plans/{reseller_code}"),
            Some(&admin_token),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Inactive plans disappear from the customer pricing list only.
    let (status, plans) = send(&t.app, req("GET", "/api/billing/plans", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(plans["plans"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["code"] != reseller_code));
}

#[tokio::test]
async fn session_cookie_csrf_and_refresh_rotation() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let email = unique_email();
    let (status, body) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "name": "Cookie", "email": email.clone(), "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register: {body}");
    let access = body["access"].as_str().expect("access token").to_string();
    let user_id = body["user"]["id"].as_str().expect("user id").to_string();
    // The refresh half travels only in the HttpOnly cookie, never the body.
    assert!(
        body["refresh"].is_null(),
        "refresh must not leak in the JSON body"
    );

    // Login sets an HttpOnly, SameSite=Lax session cookie.
    let (status, set_cookie, login) = send_headers(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login: {login}");
    assert_eq!(login["user"]["id"].as_str(), Some(user_id.as_str()));
    assert!(login["refresh"].is_null(), "login must not expose refresh");
    let cookie = set_cookie
        .as_deref()
        .expect("login must set a session cookie");
    assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
    assert!(
        cookie.contains("SameSite=Lax"),
        "cookie must be SameSite=Lax"
    );
    assert!(cookie.contains("Path=/"), "cookie must be Path=/");
    let session = session_cookie(cookie);

    // The session cookie is the only way to call refresh.
    let (status, _) = send(&t.app, req("POST", "/api/auth/refresh", None, None)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "refresh without cookie must fail"
    );

    // Refresh with the cookie rotates both halves and never returns refresh.
    let mut refresh_req = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut refresh_req, &session);
    let (status, rotated_cookie, rotated) = send_headers(&t.app, refresh_req).await;
    assert_eq!(status, StatusCode::OK, "cookie refresh: {rotated}");
    assert!(rotated["refresh"].is_null());
    assert!(rotated["access"].as_str().is_some());
    let rotated_session = session_cookie(rotated_cookie.as_deref().expect("rotated cookie"));
    assert_ne!(rotated_session, session, "refresh must rotate the cookie");

    // A replayed (old) cookie is treated as theft: the session family dies.
    let mut replay_req = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut replay_req, &session);
    let (status, _) = send(&t.app, replay_req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "replay must be rejected");
    // The new cookie survives replay detection untouched? No: reuse revokes the
    // whole user family, so the rotated cookie is dead too.
    let mut after_replay = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut after_replay, &rotated_session);
    let (status, _) = send(&t.app, after_replay).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "family revoked after replay"
    );

    // CSRF probe: a cross-site POST carrying a valid cookie + forged Origin is
    // blocked even though the cookie rides along.
    let (status, login2, body2) = login_probe(&t.app, &email).await;
    assert_eq!(status, StatusCode::OK, "login: {body2}");
    let cookie2 = login2.as_deref().expect("login sets a cookie");
    let session2 = session_cookie(cookie2);
    let mut csrf = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut csrf, &session2);
    csrf.headers_mut().insert(
        header::ORIGIN,
        "http://evil.example"
            .parse::<axum::http::HeaderValue>()
            .unwrap(),
    );
    let (status, _) = send(&t.app, csrf).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cross-site refresh must be blocked"
    );

    let mut untrusted_local = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut untrusted_local, &session2);
    untrusted_local.headers_mut().insert(header::ORIGIN, "http://localhost:18081".parse().unwrap());
    let (status, _) = send(&t.app, untrusted_local).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "local origin without proxy attestation must be blocked");

    let mut trusted_local = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut trusted_local, &session2);
    trusted_local.headers_mut().insert(header::ORIGIN, "http://localhost:18081".parse().unwrap());
    trusted_local.headers_mut().insert("x-cs-admin-local", "1".parse().unwrap());
    let (status, local_cookie, _) = send_headers(&t.app, trusted_local).await;
    assert_eq!(status, StatusCode::OK, "trusted SSH admin origin must refresh the session");
    let local_session = session_cookie(local_cookie.as_deref().expect("rotated local cookie"));

    // Logout clears the cookie and revokes remaining sessions.
    let mut logout_req = req("POST", "/api/auth/logout", Some(&access), None);
    with_cookie(&mut logout_req, &local_session);
    let (status, cleared, _) = send_headers(&t.app, logout_req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        cleared.as_deref().unwrap_or("").contains("Max-Age=0"),
        "logout must clear the cookie"
    );
    let mut after_logout = req("POST", "/api/auth/refresh", None, None);
    with_cookie(&mut after_logout, &session2);
    let (status, _) = send(&t.app, after_logout).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "refresh after logout must fail"
    );
}

/// WS5.4 self-service erasure, proven as a blocking round-trip: register,
/// then attempt deletion with the wrong password (must NOT erase, 401), retry
/// with the correct password (account gone, `ok:true`), relogin is rejected
/// because the row no longer exists, and the audit CSV carries the
/// `account.erase` event — the customer-side mirror of the admin erase leg.
#[tokio::test]
async fn customer_self_service_erasure() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, _user_id, email) = register(&t.app).await;
    let password = "Str0ng-Pass!23";

    // Wrong password is refused with 401: a stolen bearer token alone cannot
    // destroy the account (DELETE-equivalent POST is refused).
    let (status, body) = send(
        &t.app,
        req(
            "POST",
            "/api/account/delete",
            Some(&access),
            Some(json!({ "password": "Definitely-Wrong!1" })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "wrong password must not erase: {body}"
    );

    // The user is still alive post-401 typo: a profile fetch succeeds.
    let (status, me) = send(&t.app, req("GET", "/api/me", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK, "profile after wrong password: {me}");

    // Correct password erases the account irreversibly.
    let (status, erased) = send(
        &t.app,
        req(
            "POST",
            "/api/account/delete",
            Some(&access),
            Some(json!({ "password": password })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "self erase: {erased}");
    assert_eq!(erased["ok"], true, "erase body: {erased}");

    // Platform-only accounts have no provider mailbox, so self-erasure must
    // not fabricate a Stalwart deletion job from the login email.
    let delete_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provisioning_jobs
         WHERE target_email = $1 AND operation = 'delete_mailbox'",
    )
    .bind(&email)
    .fetch_one(&t.db)
    .await
    .expect("platform identity deletion job count");
    assert_eq!(delete_jobs, 0);

    // The access token is now inert — the row it referenced is gone.
    let (status, _) = send(&t.app, req("GET", "/api/me", Some(&access), None)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "token must die with the account"
    );

    // Relogin with the same (correct) credential is rejected: no row remains.
    let (status, relogin) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": password })),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "relogin after self-erase: {relogin}"
    );
}

/// Login a user and return (Set-Cookie header, body) for the cookie probes.
async fn login_probe(app: &Router, email: &str) -> (StatusCode, Option<String>, Value) {
    send_headers(
        app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await
}

#[tokio::test]
async fn two_factor_enrollment_and_login_flow() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, _user_id, email) = register(&t.app).await;

    let (status, setup) = send(
        &t.app,
        req(
            "POST",
            "/api/account/2fa/setup",
            Some(&access),
            Some(json!({ "current_password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "2fa setup: {setup}");
    assert!(setup["qr_svg"].as_str().is_some_and(|svg| svg.contains("<svg")));
    let secret = setup["secret"].as_str().expect("setup secret");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = cs_mail_api::services::two_factor::code_at(secret, now).expect("totp code");

    let (status, confirmed) = send(
        &t.app,
        req(
            "POST",
            "/api/account/2fa/confirm",
            Some(&access),
            Some(json!({ "code": code })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "2fa confirm: {confirmed}");
    let recovery_codes = confirmed["recovery_codes"]
        .as_array()
        .expect("recovery codes")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(recovery_codes.len(), 10);

    let (status, challenge) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "2fa login challenge: {challenge}");
    assert_eq!(challenge["two_factor_required"], true);
    let challenge_token = challenge["challenge_token"].as_str().unwrap().to_string();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = cs_mail_api::services::two_factor::code_at(secret, now).expect("totp code");
    let (status, set_cookie, verified) = send_headers(
        &t.app,
        req(
            "POST",
            "/api/auth/2fa/verify",
            None,
            Some(json!({ "challenge_token": challenge_token, "code": code })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "2fa verify: {verified}");
    assert!(verified["access"].as_str().is_some());
    assert!(set_cookie.as_deref().is_some_and(|value| value.contains("cs_mail_session=")));

    // A completed challenge is one-time even when the code is still within its
    // valid 30-second TOTP window.
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/2fa/verify",
            None,
            Some(json!({ "challenge_token": challenge["challenge_token"], "code": code })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Recovery codes are accepted once and reduce the remaining-code count.
    let (status, challenge) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "Str0ng-Pass!23" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // A successful TOTP time-step cannot be replayed through a fresh login
    // challenge while the same code remains within the verifier drift window.
    let (status, _) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/2fa/verify",
            None,
            Some(json!({
                "challenge_token": challenge["challenge_token"],
                "code": code
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, recovered) = send(
        &t.app,
        req(
            "POST",
            "/api/auth/2fa/verify",
            None,
            Some(json!({
                "challenge_token": challenge["challenge_token"],
                "code": recovery_codes[0]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "2fa recovery login: {recovered}");
    assert_eq!(recovered["recovery_code_used"], true);
    let recovered_access = recovered["access"].as_str().unwrap();
    let (status, factor_status) = send(
        &t.app,
        req("GET", "/api/account/2fa/status", Some(recovered_access), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(factor_status["recovery_codes_remaining"], 9);
}

#[tokio::test]
async fn sender_identity_and_idempotent_draft_flow() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, user_id, email) = register(&t.app).await;
    attach_test_mailbox(&t.db, &user_id, &email).await;
    let (status, body) = send(&t.app, req("GET", "/api/identities", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK, "identities failed: {body}");
    let identities = body["identities"].as_array().expect("identity array");
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0]["email"].as_str(), Some(email.as_str()));
    assert_eq!(identities[0]["primary"].as_bool(), Some(true));
    let identity_id = identities[0]["id"].as_str().expect("identity id");

    let (status, updated) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/identities/{identity_id}"),
            Some(&access),
            Some(json!({
                "display_name": "Production Sender",
                "reply_to": email,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "identity update failed: {updated}");
    assert_eq!(updated["display_name"].as_str(), Some("Production Sender"));

    let client_key = Uuid::new_v4();
    let first_payload = json!({
        "client_key": client_key,
        "identity_id": identity_id,
        "subject": "autosave one",
        "body_text": "first body",
        "to": [], "cc": [], "bcc": [], "attachments": []
    });
    let (status, first) = send(
        &t.app,
        req("POST", "/api/drafts", Some(&access), Some(first_payload)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "draft create failed: {first}");
    let draft_id = first["id"].as_str().expect("draft id").to_string();
    let send_key = first["send_key"].as_str().expect("server send key").to_string();

    let second_payload = json!({
        "client_key": client_key,
        "identity_id": identity_id,
        "subject": "autosave two",
        "body_text": "latest body",
        "to": [], "cc": [], "bcc": [], "attachments": []
    });
    let (status, second) = send(
        &t.app,
        req("POST", "/api/drafts", Some(&access), Some(second_payload)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "idempotent draft save failed: {second}");
    assert_eq!(second["id"].as_str(), Some(draft_id.as_str()));
    assert_eq!(second["subject"].as_str(), Some("autosave two"));
    assert_eq!(second["send_key"].as_str(), Some(send_key.as_str()));

    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM mail_drafts WHERE client_key = $1",
    )
    .bind(client_key)
    .fetch_one(&t.db)
    .await
    .expect("count idempotent draft rows");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn launch_certification_ledger_rejects_false_pass() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let release_hash = "a".repeat(64);
    let report_hash = "b".repeat(64);
    let ok = sqlx::query(
        "INSERT INTO launch_certification_runs(
           release_label,release_sha256,status,report_sha256,report_path,
           mandatory_passed,mandatory_failed,completed_at
         ) VALUES ('integration-certification',$1,'passed',$2,'/tmp/report.json',12,0,now())",
    )
    .bind(&release_hash)
    .bind(&report_hash)
    .execute(&t.db)
    .await;
    assert!(ok.is_ok(), "valid passed certification should be accepted: {ok:?}");

    let false_pass = sqlx::query(
        "INSERT INTO launch_certification_runs(
           release_label,release_sha256,status,report_sha256,report_path,
           mandatory_passed,mandatory_failed,completed_at
         ) VALUES ('integration-false-pass',$1,'passed',$2,'/tmp/report.json',11,1,now())",
    )
    .bind(&release_hash)
    .bind(&report_hash)
    .execute(&t.db)
    .await;
    assert!(false_pass.is_err(), "a passed certification with mandatory failures must be rejected");
}
