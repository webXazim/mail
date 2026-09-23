use axum::http::StatusCode;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit;
use crate::error::ApiError;
use crate::services::mime;
use crate::state::AppState;

const VERIFY_TTL_SECS: i64 = 24 * 3600;
const RESET_TTL_SECS: i64 = 15 * 60;

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Opaque single-use token: 256 bits of hex, stored only as a sha256 hash.
pub fn random_token() -> String {
    let a = Uuid::new_v4().as_simple().to_string();
    let b = Uuid::new_v4().as_simple().to_string();
    format!("{a}{b}")
}

async fn submit_system_mail(state: &AppState, outgoing: &mime::Outgoing) -> Result<(), ApiError> {
    let result = if let Some(mailer) = &state.system_mailer {
        mailer.send(outgoing).await
    } else {
        let bytes = outgoing
            .build()
            .map_err(|e| ApiError::internal(format!("MIME build failed: {e}")))?;
        let recipients = outgoing.to.iter().chain(outgoing.cc.iter())
            .map(|address| address.email.clone()).collect::<Vec<_>>();
        state.stalwart.submit_raw(&outgoing.from.email, &recipients, &bytes).await
            .map(|_| ())
            .map_err(|error| ApiError::new(StatusCode::BAD_GATEWAY, "mail_submission", error.public_message()))
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if state.return_token_links => {
            tracing::warn!(error=%error.message, "system email skipped in development mode");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

async fn persist_token(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    token: &str,
    ttl_secs: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO email_tokens (user_id, token_hash, kind, expires_at)
         VALUES ($1, $2, $3, now() + interval '1 second' * $4)",
    )
    .bind(user_id)
    .bind(token_hash(token))
    .bind(kind)
    .bind(ttl_secs)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(())
}

/// Issue a verification token and deliver the email.
/// Returns the click-through URL (to the SPA /verify-email route).
pub async fn send_verification(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let token = random_token();
    persist_token(state, user_id, "verify", &token, VERIFY_TTL_SECS).await?;
    let link = format!("{}/verify-email?token={token}", state.public_origin);
    let subject = if display_name.is_empty() {
        "Verify your email".into()
    } else {
        format!("Verify your email, {display_name}")
    };
    deliver(state, &subject, email, display_name, &link).await?;
    audit::record(state, Some(user_id), "auth.verify.token_issued", json!({})).await;
    Ok(link)
}

pub async fn send_password_reset(
    state: &AppState,
    user_id: Uuid,
    email: &str,
    display_name: &str,
) -> Result<String, ApiError> {
    let token = random_token();
    persist_token(state, user_id, "reset", &token, RESET_TTL_SECS).await?;
    let link = format!("{}/reset-password?token={token}", state.public_origin);
    let subject = if display_name.is_empty() {
        "Reset your password".into()
    } else {
        format!("Reset your password, {display_name}")
    };
    deliver(state, &subject, email, display_name, &link).await?;
    audit::record(
        state,
        Some(user_id),
        "auth.password_reset.token_issued",
        json!({}),
    )
    .await;
    Ok(link)
}

/// Build and submit a one-off transactional email via Mailer when configured,
/// or the existing SMTP relay otherwise. The sender is a service address on the managed
/// default domain. Login/contact identities are no longer assumed to be local
/// mailboxes, so verification and reset messages must be deliverable to an
/// arbitrary external address without spoofing that recipient as the sender.
/// Delivery is best-effort only in explicit development mode
/// (`return_token_links`) and load-bearing in production.
async fn deliver(
    state: &AppState,
    subject: &str,
    to_email: &str,
    display_name: &str,
    link: &str,
) -> Result<(), ApiError> {
    let from_email = format!("mailer@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("CS Mail".to_string()),
            email: from_email.clone(),
        },
        to: vec![mime::Address {
            name: (!display_name.is_empty()).then(|| display_name.to_string()),
            email: to_email.to_string(),
        }],
        cc: vec![],
        reply_to: None,
        subject: subject.to_string(),
        body_text: format!(
            "Use this link to continue:\n\n{link}\n\nIf you did not request this, you can ignore this email.\n"
        ),
        body_html: Some(format!(
            "<p>Use the button below to continue:</p>\
             <p><a href=\"{link}\">{link}</a></p>\
             <p>If you did not request this, you can ignore this email.</p>\n"
        )),
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}

/// Deliver a forwarding-destination verification code to an arbitrary target.
/// The envelope sender is a local CS Mail service address so production SMTP
/// can DKIM-sign/relay it without impersonating the external recipient.
pub async fn send_forwarding_verification(
    state: &AppState,
    target_email: &str,
    code: &str,
) -> Result<(), ApiError> {
    let from_email = format!("mailer@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("CS Mail".to_string()),
            email: from_email.clone(),
        },
        to: vec![mime::Address { name: None, email: target_email.to_string() }],
        cc: vec![],
        reply_to: None,
        subject: "Confirm mail forwarding".to_string(),
        body_text: format!(
            "Use this verification code in CS Mail to confirm forwarding to this address:\n\n{code}\n\nThe code expires in 30 minutes. If you did not request this, ignore this message.\n"
        ),
        body_html: Some(format!(
            "<p>Use this verification code in CS Mail to confirm forwarding to this address:</p>\
             <p><strong>{code}</strong></p>\
             <p>The code expires in 30 minutes. If you did not request this, ignore this message.</p>"
        )),
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}

/// Deliver a sender-identity ownership verification code to an address that is
/// not already a CS Mail mailbox/alias. A verified external identity can be
/// selected as the RFC 5322 From address, but only after this challenge is
/// completed server-side.
pub async fn send_sender_identity_verification(
    state: &AppState,
    target_email: &str,
    code: &str,
) -> Result<(), ApiError> {
    let from_email = format!("mailer@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("CS Mail".to_string()),
            email: from_email.clone(),
        },
        to: vec![mime::Address { name: None, email: target_email.to_string() }],
        cc: vec![],
        reply_to: None,
        subject: "Confirm sender identity".to_string(),
        body_text: format!(
            "Use this verification code in CS Mail to confirm that you can send as {target_email}:\n\n{code}\n\nThe code expires in 30 minutes. If you did not request this, ignore this message.\n"
        ),
        body_html: Some(format!(
            "<p>Use this verification code in CS Mail to confirm that you can send as <strong>{target_email}</strong>:</p>\
             <p><strong>{code}</strong></p>\
             <p>The code expires in 30 minutes. If you did not request this, ignore this message.</p>"
        )),
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}

/// Deliver a support-agent reply to the ticket requester. Delivery failure is
/// surfaced to the agent before the reply is committed to ticket history, so
/// the support UI never claims a customer-visible email was sent when it was not.
pub async fn send_support_reply(
    state: &AppState,
    target_email: &str,
    requester_name: &str,
    reference: &str,
    subject: &str,
    message: &str,
) -> Result<(), ApiError> {
    let from_email = format!("support@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("CS Mail Support".to_string()),
            email: from_email.clone(),
        },
        to: vec![mime::Address {
            name: (!requester_name.is_empty()).then(|| requester_name.to_string()),
            email: target_email.to_string(),
        }],
        cc: vec![],
        reply_to: Some(mime::Address { name: Some("CS Mail Support".to_string()), email: from_email.clone() }),
        subject: format!("Re: [{reference}] {subject}"),
        body_text: format!(
            "CS Mail Support\nTicket {reference}\n\n{message}\n\nIf you need to add more information, contact support again and include this ticket reference.\n"
        ),
        body_html: None,
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}

/// Deliver a business-membership invitation. The login email is intentionally
/// independent from any hosted mailbox, so invitations can be sent to Gmail,
/// Outlook, or another provider before the business domain is onboarded.
pub async fn send_business_invitation(
    state: &AppState,
    target_email: &str,
    organization_name: &str,
    link: &str,
) -> Result<(), ApiError> {
    let from_email = format!("mailer@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address {
            name: Some("CS Mail".to_string()),
            email: from_email.clone(),
        },
        to: vec![mime::Address { name: None, email: target_email.to_string() }],
        cc: vec![],
        reply_to: None,
        subject: format!("Join {organization_name} on CS Mail"),
        body_text: format!(
            "You were invited to join {organization_name} on CS Mail.\n\nAccept the invitation:\n{link}\n\nThe invitation expires in 7 days. If you were not expecting this invitation, ignore this message.\n"
        ),
        body_html: Some(format!(
            "<p>You were invited to join <strong>{organization_name}</strong> on CS Mail.</p>\
             <p><a href=\"{link}\">Accept invitation</a></p>\
             <p>The invitation expires in 7 days. If you were not expecting this invitation, ignore this message.</p>"
        )),
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}

/// Deliver an invitation that reserves and assigns a real business mailbox.
/// The recipient still authenticates with their platform login address; the
/// hosted mailbox address is bound only after the single-use token is accepted.
pub async fn send_mailbox_invitation(
    state: &AppState,
    target_email: &str,
    organization_name: &str,
    mailbox_address: &str,
    link: &str,
) -> Result<(), ApiError> {
    let subject = format!("Your {mailbox_address} mailbox is ready to claim");
    let display = if organization_name.trim().is_empty() { "CS Mail" } else { organization_name };
    deliver(state, &subject, target_email, display, link).await
}

/// Deliver an issued invoice or payment receipt from the durable billing
/// outbox. The invoice itself is the immutable database snapshot; email is a
/// notification/delivery channel and can be retried without changing amounts.
pub async fn send_billing_document(
    state: &AppState,
    target_email: &str,
    order: &crate::services::billing::OrderView,
    kind: &str,
) -> Result<(), ApiError> {
    let invoice = order.invoice_number.as_deref().unwrap_or("Invoice");
    let currency = order.currency.to_uppercase();
    let money = |cents: i64| format!("{currency} {}.{:02}", cents / 100, cents.abs() % 100);
    let due = order
        .due_at
        .map(|value| value.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "See Billing".to_string());
    let paid = kind == "payment_received" || order.invoice_status == "paid";
    let subject = if paid {
        format!("Payment received — {invoice}")
    } else {
        format!("Invoice {invoice} — {}", order.plan_name)
    };
    let status_line = if paid {
        "Payment status: Paid".to_string()
    } else {
        format!("Payment status: Due by {due}")
    };
    let billing_url = format!("{}/mail/billing/invoices/{}", state.public_origin, invoice);
    let from_email = format!("mailer@{}", state.stalwart.default_domain());
    let outgoing = mime::Outgoing {
        from: mime::Address { name: Some("CS Mail Billing".to_string()), email: from_email.clone() },
        to: vec![mime::Address { name: None, email: target_email.to_string() }],
        cc: vec![],
        reply_to: Some(mime::Address { name: Some("CS Mail Billing".to_string()), email: from_email.clone() }),
        subject,
        body_text: format!(
            "CS Mail billing\n\nInvoice: {invoice}\nBusiness: {}\nPlan: {}\nMailboxes: {} total ({} included, {} additional)\nBase plan: {}\nAdditional mailbox rate: {} each\nSubtotal: {}\nTax: {}\nTotal: {}\n{}\n\nView, print or save the invoice as PDF:\n{}\n\nPayment method: {}\n",
            order.organization_name,
            order.plan_name,
            order.mailbox_count,
            order.included_mailbox_count,
            order.extra_mailbox_count,
            money(order.base_price_cents),
            money(order.extra_mailbox_unit_price_cents),
            money(order.subtotal_cents),
            money(order.tax_cents),
            money(order.total_cents),
            status_line,
            billing_url,
            order.payment_method,
        ),
        body_html: None,
        attachments: vec![],
        in_reply_to: None,
        references: vec![],
        list_unsubscribe: None,
        message_id_local: Uuid::new_v4().as_simple().to_string(),
        domain: state.stalwart.default_domain().to_string(),
    };
    submit_system_mail(state, &outgoing).await
}
