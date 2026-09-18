use crate::handlers;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::{routing::get, Router};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

/// Maximum accepted request body. JSON payloads here are small (contacts,
/// events, settings), and this guards against oversized/malformed uploads.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

pub fn build_router(state: AppState) -> Router {
    let origins: Vec<_> = state
        .cors_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    let cors = if origins.is_empty() {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let public = Router::new()
        .route("/api/health", get(handlers::health::health))
        // WS6.3: Prometheus scrape target (restrict at the reverse proxy).
        .route("/api/metrics", get(handlers::metrics::metrics))
        .route("/api/ws", get(crate::ws::ws_or_poll))
        // WS7.2: token-signed one-click unsubscribe (no session).
        .route(
            "/api/unsubscribe",
            get(handlers::unsubscribe::unsubscribe).post(handlers::unsubscribe::unsubscribe),
        )
        .merge(handlers::auth::routes());

    let protected = Router::new()
        .route(
            "/api/profile",
            get(handlers::profile::get).put(handlers::profile::update),
        )
        // WS3.6 admin surface (role-gated).
        .route("/api/admin/overview", get(handlers::admin::overview))
        .route("/api/admin/users", get(handlers::admin::users))
        .route(
            "/api/admin/users/:id",
            axum::routing::patch(handlers::admin::update_user).delete(handlers::admin::delete_user),
        )
        .route("/api/admin/aliases", get(handlers::admin::aliases))
        .route("/api/admin/audit", get(handlers::admin::audit))
        .route(
            "/api/admin/audit/export",
            get(handlers::admin::audit_export),
        )
        .route(
            "/api/admin/suppressions",
            get(handlers::admin::suppressions).post(handlers::admin::add_suppression),
        )
        .route(
            "/api/admin/suppressions/dsn",
            axum::routing::post(handlers::admin::ingest_dsn),
        )
        .route(
            "/api/admin/suppressions/:email",
            axum::routing::delete(handlers::admin::remove_suppression),
        )
        .route(
            "/api/auth/logout",
            axum::routing::post(handlers::auth::logout),
        )
        // WS5.4 self-service erasure (password re-confirmed in the handler).
        .route(
            "/api/account/delete",
            axum::routing::post(handlers::account::delete_self),
        )
        .route(
            "/api/contacts",
            get(handlers::contacts::list).post(handlers::contacts::create),
        )
        .route(
            "/api/contacts/:id",
            axum::routing::put(handlers::contacts::update).delete(handlers::contacts::delete),
        )
        .route(
            "/api/aliases",
            get(handlers::aliases::list).post(handlers::aliases::create),
        )
        .route(
            "/api/aliases/:id",
            axum::routing::delete(handlers::aliases::delete),
        )
        .route(
            "/api/settings",
            get(handlers::settings::get).put(handlers::settings::put),
        )
        .route(
            "/api/calendar/events",
            get(handlers::calendar::list).post(handlers::calendar::create),
        )
        .route(
            "/api/calendar/events/:id",
            axum::routing::put(handlers::calendar::update).delete(handlers::calendar::delete),
        )
        // WS2 mailbox endpoints (JMAP-backed read path).
        .route("/api/mail/mailboxes", get(handlers::mailbox::mailboxes))
        .route("/api/mail/threads", get(handlers::mailbox::threads))
        .route(
            "/api/mail/threads/state",
            axum::routing::post(handlers::mailbox::set_state),
        )
        .route(
            "/api/mail/threads/move",
            axum::routing::post(handlers::mailbox::move_emails),
        )
        .route(
            "/api/mail/threads/destroy",
            axum::routing::post(handlers::mailbox::destroy),
        )
        .route("/api/mail/search", get(handlers::mailbox::search))
        .route(
            "/api/mail/thread/:thread_id",
            get(handlers::mailbox::thread),
        )
        .route(
            "/api/mail/attachment/:blob_id",
            get(handlers::mailbox::attachment),
        )
        // WS2.3 compose/send/drafts.
        .route("/api/send", axum::routing::post(handlers::send::send))
        .route(
            "/api/drafts",
            get(handlers::send::drafts).post(handlers::send::create_draft),
        )
        .route(
            "/api/drafts/:id",
            axum::routing::get(handlers::send::get_draft)
                .put(handlers::send::update_draft)
                .delete(handlers::send::delete_draft_handler),
        )
        // WS3.3 scheduled sends + read receipts.
        .route(
            "/api/scheduled",
            get(handlers::schedule::list).post(handlers::schedule::create),
        )
        .route(
            "/api/scheduled/:id",
            axum::routing::delete(handlers::schedule::delete),
        )
        .route(
            "/api/receipts",
            get(handlers::receipts::list).post(handlers::receipts::record),
        )
        .route(
            "/api/receipt-requests",
            get(handlers::receipts::list_requests).post(handlers::receipts::request),
        )
        // WS4 manual-payment billing (customer side).
        .route("/api/billing", get(handlers::billing::summary))
        .route("/api/billing/plans", get(handlers::billing::plans))
        .route(
            "/api/billing/orders",
            axum::routing::post(handlers::billing::create_order),
        )
        .route(
            "/api/billing/orders/:id/paid",
            axum::routing::post(handlers::billing::submit_paid),
        )
        .route(
            "/api/billing/orders/:id/cancel",
            axum::routing::post(handlers::billing::cancel_order),
        )
        .route("/api/billing/invoices", get(handlers::billing::invoices))
        // WS4 admin payment controls.
        .route("/api/admin/plans", get(handlers::billing::admin_plans))
        .route(
            "/api/admin/plans",
            axum::routing::post(handlers::billing::admin_create_plan),
        )
        .route(
            "/api/admin/plans/:code",
            axum::routing::patch(handlers::billing::admin_update_plan)
                .delete(handlers::billing::admin_deactivate_plan),
        )
        .route("/api/admin/orders", get(handlers::billing::admin_orders))
        .route(
            "/api/admin/orders/:id/approve",
            axum::routing::post(handlers::billing::admin_approve_order),
        )
        .route(
            "/api/admin/orders/:id/reject",
            axum::routing::post(handlers::billing::admin_reject_order),
        )
        .route(
            "/api/admin/billing-settings",
            get(handlers::billing::admin_settings).put(handlers::billing::admin_update_settings),
        );

    let api = public.merge(protected);

    let metrics_layer = axum::middleware::from_fn_with_state(
        state.metrics.clone(),
        crate::middleware::metrics::track,
    );

    // Layer order (last call = outermost). CORS outermost so every response,
    // including rejection/error responses, carries the allow-headers the
    // browser needs; metrics records final status/latency; body limit tracks
    // uploads; TraceLayer logs all calls.
    api.layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(metrics_layer)
        .layer(cors)
        .with_state(state)
}
