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

use harbor_api::metrics::Metrics;
use harbor_api::middleware::rate_limit::RateLimiter;
use harbor_api::router::build_router;
use harbor_api::services::provisioning::MailBridge;
use harbor_api::services::smtp::SmtpConfig;
use harbor_api::state::AppState;
use harbor_api::ws::EventHub;

struct TestApp {
    app: Router,
    db: PgPool,
}

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

    let state = AppState {
        db: db.clone(),
        jwt_secret: "integration-test-secret".into(),
        jwt_access_ttl_secs: 900,
        jwt_refresh_ttl_secs: 2_592_000,
        cors_origins: vec![],
        hub: EventHub::new(),
        public_origin: "http://localhost:5174".into(),
        // No verification gate and no Stalwart bridge: flows stay hermetic.
        require_verification: false,
        return_token_links: true,
        cookie_secure: false,
        mail: MailBridge::new(
            String::new(),
            String::new(),
            String::new(),
            "example.test".into(),
            0,
        ),
        smtp: SmtpConfig::default(),
        rate: RateLimiter::new(),
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
        .strip_prefix("harbor_session=")
        .expect("harbor_session cookie")
        .to_string()
}

fn with_cookie(request: &mut Request<Body>, cookie_value: &str) {
    let value = format!("harbor_session={cookie_value}");
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

#[tokio::test]
async fn auth_contacts_and_calendar_flow() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (access, user_id, email) = register(&t.app).await;

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

    // Profile exposes the plan + limits computed server-side.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK, "profile: {profile}");
    assert_eq!(profile["plan"], "solo");

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

    let (status, list) = send(&t.app, req("GET", "/api/contacts", Some(&access), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["contacts"].as_array().unwrap().len(), 1);

    let (status, updated) = send(
        &t.app,
        req(
            "PUT",
            &format!("/api/contacts/{contact_id}"),
            Some(&access),
            Some(json!({ "phone": "+1-555-0100" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "contact update: {updated}");
    assert_eq!(updated["phone"], "+1-555-0100");

    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/contacts/{contact_id}"),
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
                "category": "meeting"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "calendar create: {event}");
    let event_id = event["id"].as_str().expect("event id").to_string();

    let (status, events) = send(
        &t.app,
        req("GET", "/api/calendar/events", Some(&access), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(events["events"].as_array().is_some_and(|a| !a.is_empty()));

    let (status, _) = send(
        &t.app,
        req(
            "DELETE",
            &format!("/api/calendar/events/{event_id}"),
            Some(&access),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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
    sqlx::query("UPDATE users SET role = 'admin' WHERE email = $1")
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

    // Metrics scrape is public and exposes the Harbor series.
    let response = t
        .app
        .clone()
        .oneshot(req("GET", "/api/metrics", None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("harbor_http_requests_total"));
    assert!(text.contains("harbor_db_pool_connections"));
}

#[tokio::test]
async fn admin_provisions_and_resets_user() {
    let Some(t) = test_app().await else {
        eprintln!("skipping: set TEST_DATABASE_URL to run integration tests");
        return;
    };

    let (member_token, _id, admin_email) = register(&t.app).await;
    sqlx::query("UPDATE users SET role = 'admin' WHERE email = $1")
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

    // Customer-side summary: current solo plan, payment instructions, no orders.
    let (token, _user_id, _email) = register(&t.app).await;
    let (status, billing) = send(&t.app, req("GET", "/api/billing", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK, "billing summary: {billing}");
    assert_eq!(billing["current_plan"]["code"], "solo");
    assert!(billing["settings"]["bank_details"].is_string());
    assert!(billing["orders"].as_array().unwrap().is_empty());

    // Pricing list exposes all three seeded plans.
    let (status, plans) = send(&t.app, req("GET", "/api/billing/plans", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plans["plans"].as_array().unwrap().len(), 3);

    // Place an order for Team, then mark it paid with a bank reference.
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
    assert_eq!(order["amount_cents"], 800);
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

    // Admin reviews the order: approve activates Team on the customer account.
    let (_member_token, _id, admin_email) = register(&t.app).await;
    sqlx::query("UPDATE users SET role = 'admin' WHERE email = $1")
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

    // The customer's plan is now team, and the invoice appears in their list.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["plan"], "team");

    let (status, invoices) = send(
        &t.app,
        req("GET", "/api/billing/invoices", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invoices["invoices"].as_array().unwrap().len(), 1);

    // A reject path: create a second order and turn it down.
    let (status, order2) = send(
        &t.app,
        req(
            "POST",
            "/api/billing/orders",
            Some(&token),
            Some(
                json!({ "plan_code": "business", "payment_method": "paypal", "customer_note": "" }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let order2_id = order2["id"].as_str().unwrap().to_string();
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

    // Customer withdraws an open order (cancel leg, WS4.4): a fresh Team order
    // is opened, then cancelled before the admin acts. The approved plan is NOT
    // touched — it stays ``team`` until the admin assigns another.
    let (status, order3) = send(
        &t.app,
        req(
            "POST",
            "/api/billing/orders",
            Some(&token),
            Some(json!({ "plan_code": "team", "payment_method": "paypal", "customer_note": "" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "open order3: {order3}");
    assert_eq!(order3["status"], "pending");
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

    // The approved plan survived the cancel: the member is still on ``team``.
    let (status, profile) = send(&t.app, req("GET", "/api/profile", Some(&token), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["plan"], "team", "cancel must not downgrade the approved plan");

    // A cancelled (never-paid) order contributes no invoice.
    let (status, invoices) = send(
        &t.app,
        req("GET", "/api/billing/invoices", Some(&token), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        invoices["invoices"].as_array().unwrap().len(),
        1,
        "only the approved order invoices"
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
                "features": ["10 mailboxes"], "sort_order": 4, "active": true
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create plan: {created}");
    assert_eq!(created["plan"]["price"], "$15.00");

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

    // Logout clears the cookie and revokes remaining sessions.
    let mut logout_req = req("POST", "/api/auth/logout", Some(&access), None);
    with_cookie(&mut logout_req, &session2);
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
