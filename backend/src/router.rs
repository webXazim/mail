use crate::handlers;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::{routing::get, Router};
use axum::http::header::HeaderName;
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
            .expose_headers([
                HeaderName::from_static("x-request-id"),
                HeaderName::from_static("x-cs-contract-version"),
            ])
    } else {
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(Any)
            .allow_headers(Any)
            .expose_headers([
                HeaderName::from_static("x-request-id"),
                HeaderName::from_static("x-cs-contract-version"),
            ])
    };

    let public = Router::new()
        .route("/api/health", get(handlers::health::health))
        .route("/api/health/live", get(handlers::health::live))
        .route("/api/health/ready", get(handlers::health::ready))
        .route("/api/meta", get(handlers::meta::get))
        .route("/api/public/plans", get(handlers::public_api::plans))
        .route("/api/public/status", get(handlers::public_api::status))
        .route(
            "/api/public/support/tickets",
            axum::routing::post(handlers::support::create_public),
        )
        // WS6.3: Prometheus scrape target (restrict at the reverse proxy).
        .route("/api/metrics", get(handlers::metrics::metrics))
        .route("/api/ws", get(crate::ws::ws_or_poll))
        // Upgrade 26: Thunderbird-compatible mail-client autoconfiguration.
        // Customer domains may CNAME autoconfig.<domain> to this service.
        .route(
            "/.well-known/autoconfig/mail/config-v1.1.xml",
            get(handlers::mail_clients::autoconfig),
        )
        .route(
            "/mail/config-v1.1.xml",
            get(handlers::mail_clients::autoconfig),
        )
        // WS7.2: token-signed one-click unsubscribe (no session).
        .route(
            "/api/unsubscribe",
            get(handlers::unsubscribe::unsubscribe).post(handlers::unsubscribe::unsubscribe),
        )
        // Upgrade 25: normalized provider events, authenticated by an
        // independent HMAC secret inside the handler.
        .route(
            "/api/internal/delivery-events",
            axum::routing::post(handlers::deliverability::provider_event),
        )
        .merge(handlers::auth::routes());

    let protected = Router::new()
        .route(
            "/api/profile",
            get(handlers::profile::get).put(handlers::profile::update),
        )
        .route("/api/notifications", get(handlers::notifications::list))
        .route(
            "/api/notifications/read-all",
            axum::routing::post(handlers::notifications::mark_all_read),
        )
        .route(
            "/api/notifications/:id/read",
            axum::routing::post(handlers::notifications::mark_read),
        )
        .route(
            "/api/notifications/:id",
            axum::routing::delete(handlers::notifications::dismiss),
        )
        .route("/api/account/activity", get(handlers::activity::list))
        // Upgrade 19: business tenancy foundation. A platform account may
        // belong to multiple organizations without implicitly owning a mailbox.
        .route(
            "/api/organizations",
            get(handlers::organizations::list).post(handlers::organizations::create),
        )
        .route(
            "/api/organizations/:id",
            get(handlers::organizations::get).patch(handlers::organizations::update),
        )
        .route(
            "/api/organizations/:id/activate",
            axum::routing::post(handlers::organizations::activate),
        )
        .route(
            "/api/organizations/:id/members",
            get(handlers::organizations::members),
        )
        // Upgrades 20–21: customer domain claims prove DNS ownership first, then
        // provision a marker-owned provider domain and verify mail DNS readiness.
        .route(
            "/api/organizations/:id/domains",
            get(handlers::domains::list).post(handlers::domains::create),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/verify",
            axum::routing::post(handlers::domains::verify),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/cloudflare-txt",
            axum::routing::post(handlers::domains::publish_cloudflare_challenge),
        )
        .route(
            "/api/cloudflare/oauth/config",
            get(handlers::domains::cloudflare_oauth_config),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/cloudflare-oauth/exchange",
            axum::routing::post(handlers::domains::cloudflare_oauth_exchange),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/cloudflare-mail-dns",
            axum::routing::post(handlers::domains::publish_cloudflare_mail_dns),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/challenge",
            axum::routing::post(handlers::domains::rotate),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/provision",
            axum::routing::post(handlers::domains::provision),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id/dns-check",
            axum::routing::post(handlers::domains::check_dns),
        )
        .route(
            "/api/organizations/:organization_id/domains/:domain_id",
            axum::routing::delete(handlers::domains::delete),
        )
        .route(
            "/api/organizations/:id/mailboxes",
            get(handlers::business_mailboxes::list).post(handlers::business_mailboxes::create),
        )
        .route(
            "/api/organizations/:organization_id/mailboxes/:mailbox_id",
            axum::routing::patch(handlers::business_mailboxes::update).delete(handlers::business_mailboxes::delete),
        )
        .route(
            "/api/organizations/:organization_id/mailboxes/:mailbox_id/storage",
            axum::routing::patch(handlers::business_mailboxes::update_storage),
        )
        .route(
            "/api/organizations/:organization_id/mailboxes/storage/distribute",
            axum::routing::post(handlers::business_mailboxes::distribute_available_storage),
        )
        .route(
            "/api/organizations/:organization_id/mailboxes/:mailbox_id/retry",
            axum::routing::post(handlers::business_mailboxes::retry_provisioning),
        )
        .route(
            "/api/organizations/:organization_id/mailboxes/:mailbox_id/activate",
            axum::routing::post(handlers::business_mailboxes::activate),
        )
        .route(
            "/api/organizations/:id/mailbox-invitations",
            get(handlers::business_mailboxes::invitations),
        )
        .route(
            "/api/organizations/:organization_id/mailbox-invitations/:invitation_id",
            axum::routing::delete(handlers::business_mailboxes::revoke_invitation),
        )
        .route(
            "/api/mailbox-invitations/accept",
            axum::routing::post(handlers::business_mailboxes::accept_invitation),
        )
        .route(
            "/api/organizations/:id/addresses",
            get(handlers::business_addresses::list).post(handlers::business_addresses::create),
        )
        .route(
            "/api/organizations/:organization_id/addresses/:address_id",
            axum::routing::patch(handlers::business_addresses::update).delete(handlers::business_addresses::delete),
        )
        .route(
            "/api/organizations/:id/invitations",
            get(handlers::organizations::invitations).post(handlers::organizations::invite),
        )
        .route(
            "/api/organizations/:organization_id/invitations/:invitation_id",
            axum::routing::delete(handlers::organizations::revoke_invitation),
        )
        .route(
            "/api/organization-invitations/accept",
            axum::routing::post(handlers::organizations::accept_invitation),
        )
        .route(
            "/api/support/tickets",
            get(handlers::support::my_tickets).post(handlers::support::create_authenticated),
        )
        // WS3.6 admin surface (role-gated).
        .route("/api/admin/overview", get(handlers::admin::overview))
        // Upgrade 35: localhost-only SaaS control-plane authority.
        .route("/api/admin/platform-controls", get(handlers::platform_admin::controls).put(handlers::platform_admin::update_controls))
        .route("/api/admin/businesses", get(handlers::platform_admin::businesses))
        .route("/api/admin/businesses/:id/status", axum::routing::patch(handlers::platform_admin::update_business_status))
        .route("/api/admin/businesses/:id/members", get(handlers::platform_admin::business_members))
        .route("/api/admin/businesses/:organization_id/members/:user_id", axum::routing::patch(handlers::platform_admin::update_business_member).delete(handlers::platform_admin::remove_business_member))
        .route("/api/admin/hosted-domains", get(handlers::platform_admin::domains))
        .route("/api/admin/hosted-domains/:id/action", axum::routing::post(handlers::platform_admin::domain_action))
        .route("/api/admin/hosted-mailboxes", get(handlers::platform_admin::mailboxes))
        .route("/api/admin/hosted-mailboxes/:id/action", axum::routing::post(handlers::platform_admin::mailbox_action))
        .route("/api/admin/recovery", get(handlers::platform_admin::recovery))
        .route("/api/admin/recovery/:id/retry", axum::routing::post(handlers::platform_admin::retry_recovery))
        .route("/api/admin/users", get(handlers::admin::users))
        .route(
            "/api/admin/users",
            axum::routing::post(handlers::admin::create_user),
        )
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
            "/api/admin/deliverability/events",
            get(handlers::deliverability::admin_events)
                .post(handlers::deliverability::admin_event),
        )
        .route(
            "/api/admin/deliverability/suppressions",
            get(handlers::deliverability::admin_tenant_suppressions),
        )
        .route(
            "/api/admin/deliverability/suppressions/:id",
            axum::routing::delete(handlers::deliverability::admin_remove_tenant_suppression),
        )
        .route(
            "/api/admin/deliverability/sending-controls",
            get(handlers::deliverability::admin_sending_controls)
                .put(handlers::deliverability::admin_update_sending_control),
        )
        .route(
            "/api/admin/domain",
            get(handlers::admin::domain_settings).patch(handlers::admin::update_domain_settings),
        )
        .route(
            "/api/admin/security-policy",
            get(handlers::admin::security_policy).patch(handlers::admin::update_security_policy),
        )
        .route("/api/admin/forwarders", get(handlers::admin::forwarders).post(handlers::admin::create_forwarder))
        .route(
            "/api/admin/forwarders/:id",
            axum::routing::patch(handlers::admin::update_forwarder).delete(handlers::admin::delete_forwarder),
        )
        .route(
            "/api/admin/forwarders/:id/verify",
            axum::routing::post(handlers::admin::verify_admin_forwarder),
        )
        .route("/api/admin/quarantine", get(handlers::admin::quarantine))
        .route(
            "/api/admin/quarantine/:id/release",
            axum::routing::post(handlers::admin::release_quarantine),
        )
        .route(
            "/api/admin/quarantine/:id",
            axum::routing::delete(handlers::admin::delete_quarantine),
        )
        .route("/api/admin/queue", get(handlers::admin::queue))
        .route(
            "/api/admin/queue/:id/retry",
            axum::routing::post(handlers::admin::retry_queued_message),
        )
        .route(
            "/api/admin/queue/:id",
            axum::routing::delete(handlers::admin::cancel_queued_message),
        )
        .route("/api/admin/diagnostics", get(handlers::admin::diagnostics))
        .route("/api/admin/launch-readiness", get(handlers::admin::launch_readiness))
        .route("/api/admin/launch-certifications", get(handlers::admin::launch_certifications))
        .route("/api/admin/support/tickets", get(handlers::support::admin_list))
        .route("/api/admin/support/tickets/:id", get(handlers::support::admin_get).patch(handlers::support::admin_update))
        .route("/api/admin/support/tickets/:id/reply", axum::routing::post(handlers::support::admin_reply))
        .route("/api/admin/status/incidents", get(handlers::public_api::admin_incidents).post(handlers::public_api::admin_create_incident))
        .route("/api/admin/status/incidents/:id", axum::routing::patch(handlers::public_api::admin_update_incident))
        .route(
            "/api/auth/logout",
            axum::routing::post(handlers::auth::logout),
        )
        .route(
            "/api/auth/logout-all",
            axum::routing::post(handlers::auth::logout_all),
        )
        .route(
            "/api/account/change-password",
            axum::routing::post(handlers::auth::change_password),
        )
        .route(
            "/api/account/2fa/status",
            get(handlers::two_factor::status),
        )
        .route(
            "/api/account/2fa/setup",
            axum::routing::post(handlers::two_factor::setup),
        )
        .route(
            "/api/account/2fa/confirm",
            axum::routing::post(handlers::two_factor::confirm),
        )
        .route(
            "/api/account/2fa/disable",
            axum::routing::post(handlers::two_factor::disable),
        )
        .route(
            "/api/account/2fa/recovery-codes/regenerate",
            axum::routing::post(handlers::two_factor::regenerate_recovery_codes),
        )
        .route(
            "/api/account/sessions",
            get(handlers::auth::sessions),
        )
        .route(
            "/api/account/sessions/revoke-others",
            axum::routing::post(handlers::auth::revoke_other_sessions),
        )
        .route(
            "/api/account/sessions/:id",
            axum::routing::delete(handlers::auth::revoke_session),
        )
        // WS5.4 self-service erasure (password re-confirmed in the handler).
        .route(
            "/api/account/delete",
            axum::routing::post(handlers::account::delete_self),
        )
        // Upgrade 26: external IMAP/SMTP client credentials and mailbox import.
        .route("/api/mail-clients", get(handlers::mail_clients::overview))
        .route(
            "/api/mail-clients/app-passwords",
            axum::routing::post(handlers::mail_clients::create_app_password),
        )
        .route(
            "/api/mail-clients/app-passwords/:id",
            axum::routing::delete(handlers::mail_clients::revoke_app_password),
        )
        .route(
            "/api/mail-imports",
            get(handlers::mail_clients::list_imports)
                .post(handlers::mail_clients::upload_import)
                // Upgrade 26/27: this handler streams its own bounded MBOX body
                // and enforces state.mail_import_max_bytes. The global 5 MiB
                // JSON limit must not truncate legitimate migration archives.
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/mail-imports/:id/cancel",
            axum::routing::post(handlers::mail_clients::cancel_import),
        )
        .route(
            "/api/contacts",
            get(handlers::contacts::list).post(handlers::contacts::create),
        )
        .route(
            "/api/contacts/import",
            axum::routing::post(handlers::contacts::import_csv),
        )
        .route(
            "/api/contacts/export",
            get(handlers::contacts::export_csv),
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
            "/api/calendar/import",
            axum::routing::post(handlers::calendar::import_ics),
        )
        .route(
            "/api/calendar/events/:id/ics",
            get(handlers::calendar::export_ics),
        )
        .route(
            "/api/calendar/events/:id",
            axum::routing::put(handlers::calendar::update).delete(handlers::calendar::delete),
        )
        // WS2 mailbox endpoints (JMAP-backed read path).
        .route(
            "/api/mail/mailboxes",
            get(handlers::mailbox::mailboxes).post(handlers::mailbox::create_mailbox),
        )
        .route(
            "/api/mail/mailboxes/:id",
            axum::routing::put(handlers::mailbox::update_mailbox)
                .delete(handlers::mailbox::delete_mailbox),
        )
        .route(
            "/api/mail/mailboxes/:id/empty",
            axum::routing::post(handlers::mailbox::empty_mailbox),
        )
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
        // Upgrade 12: server-authoritative incoming-mail automation.
        .route(
            "/api/mail/rules",
            get(handlers::automation::list_rules).post(handlers::automation::create_rule),
        )
        .route(
            "/api/mail/rules/reorder",
            axum::routing::put(handlers::automation::reorder_rules),
        )
        .route(
            "/api/mail/rules/:id",
            axum::routing::put(handlers::automation::update_rule)
                .delete(handlers::automation::delete_rule),
        )
        .route(
            "/api/mail/forwarding",
            get(handlers::automation::get_forwarding).put(handlers::automation::put_forwarding),
        )
        .route(
            "/api/mail/forwarding/verify",
            axum::routing::post(handlers::automation::verify_forwarding),
        )
        .route(
            "/api/mail/forwarding/resend",
            axum::routing::post(handlers::automation::resend_forwarding),
        )
        .route(
            "/api/mail/vacation",
            get(handlers::automation::get_vacation).put(handlers::automation::put_vacation),
        )
        .route(
            "/api/mail/automation/status",
            get(handlers::automation::status),
        )
        // Upgrade 07: staged outbound attachment blobs. Raw upload bodies use
        // the Body extractor and are streamed directly to persistent storage;
        // the 5 MiB JSON extractor limit therefore remains safe for all other
        // API routes.
        .route(
            "/api/attachments",
            axum::routing::post(handlers::attachments::upload),
        )
        .route(
            "/api/attachments/:id",
            get(handlers::attachments::download).delete(handlers::attachments::delete),
        )
        // WS2.3 compose/send/drafts.
        .route("/api/send", axum::routing::post(handlers::send::send))
        .route("/api/send/status/:key", get(handlers::send::send_status))
        .route(
            "/api/identities",
            get(handlers::identities::list).post(handlers::identities::create),
        )
        .route(
            "/api/identities/:id",
            axum::routing::put(handlers::identities::update)
                .delete(handlers::identities::delete),
        )
        .route(
            "/api/identities/:id/verify",
            axum::routing::post(handlers::identities::verify),
        )
        .route(
            "/api/identities/:id/resend",
            axum::routing::post(handlers::identities::resend),
        )
        .route(
            "/api/identities/:id/default",
            axum::routing::put(handlers::identities::set_default),
        )
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
            "/api/scheduled/:id/retry",
            axum::routing::post(handlers::schedule::retry_dead),
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
        .route("/api/billing/subscription/change", axum::routing::post(handlers::billing::schedule_subscription_change).delete(handlers::billing::cancel_subscription_change))
        .route("/api/billing/invoices", get(handlers::billing::invoices))
        .route("/api/billing/profile", get(handlers::billing::billing_profile).put(handlers::billing::update_billing_profile))
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
        .route("/api/admin/subscriptions", get(handlers::billing::admin_subscriptions))
        .route("/api/admin/subscriptions/:organization_id", axum::routing::patch(handlers::billing::admin_update_subscription))
        .route("/api/admin/subscriptions/:organization_id/history", get(handlers::billing::admin_subscription_history))
        .route("/api/admin/subscriptions/:organization_id/reconcile", axum::routing::post(handlers::billing::admin_reconcile_subscription))
        .route("/api/admin/subscriptions/:organization_id/retry-failures", axum::routing::post(handlers::billing::admin_retry_subscription_failures))
        .route("/api/admin/subscriptions/:organization_id/purge", axum::routing::post(handlers::billing::admin_purge_subscription_data))
        .route("/api/admin/billing-operations", get(handlers::billing::admin_billing_operations))
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

    let admin_local_layer = axum::middleware::from_fn(crate::middleware::auth::local_admin_gate);
    let contract_layer = axum::middleware::from_fn(crate::middleware::contract::headers);
    let security_layer = axum::middleware::from_fn_with_state(
        state.cookie_secure,
        crate::middleware::security::headers,
    );

    // Layer order (last call = outermost). CORS outermost so every response,
    // including rejection/error responses, carries the allow-headers the
    // browser needs; metrics records final status/latency; body limit tracks
    // uploads; TraceLayer logs all calls.
    api.layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(metrics_layer)
        .layer(admin_local_layer)
        .layer(contract_layer)
        .layer(security_layer)
        .layer(cors)
        .with_state(state)
}
