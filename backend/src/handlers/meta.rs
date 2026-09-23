use axum::Json;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CapabilityState {
    Server,
    #[allow(dead_code)]
    Partial,
    ClientOnly,
    #[allow(dead_code)]
    Planned,
}

#[derive(Debug, Serialize)]
struct Capability {
    key: &'static str,
    state: CapabilityState,
    authority: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ApiMeta {
    product: &'static str,
    api_version: &'static str,
    contract_version: u32,
    mail_backend: &'static str,
    capabilities: Vec<Capability>,
}

/// Public, non-secret API contract metadata. The frontend uses this endpoint
/// to understand which features are authoritative on the server during the
/// staged backend migration. It deliberately exposes no credentials, hostnames
/// or provider-management details.
pub async fn get() -> Json<ApiMeta> {
    Json(ApiMeta {
        product: "CS Mail",
        api_version: env!("CARGO_PKG_VERSION"),
        contract_version: 32,
        mail_backend: "self_hosted",
        capabilities: vec![
            capability("auth", CapabilityState::Server, "application+mail_server"),
            capability("profile", CapabilityState::Server, "application"),
            capability("organizations", CapabilityState::Server, "application"),
            capability("organization_memberships", CapabilityState::Server, "application"),
            capability("organization_invitations", CapabilityState::Server, "application+mail_server"),
            capability("organization_domains", CapabilityState::Server, "application+mail_server"),
            capability("domain_verification", CapabilityState::Server, "application+public_dns"),
            capability("domain_provisioning", CapabilityState::Server, "application+mail_server"),
            capability("domain_dns_readiness", CapabilityState::Server, "application+mail_server+public_dns"),
            capability("business_mailboxes", CapabilityState::Server, "application+mail_server"),
            capability("mailbox_storage_allocations", CapabilityState::Server, "application+mail_server"),
            capability("active_mailbox_context", CapabilityState::Server, "application"),
            capability("mailbox_invitations", CapabilityState::Server, "application+mail_server"),
            capability("business_aliases_groups", CapabilityState::Server, "application+mail_server"),
            capability("mailbox", CapabilityState::Server, "mail_server"),
            capability("mail_pagination", CapabilityState::Server, "mail_server"),
            capability("mail_search", CapabilityState::Server, "mail_server"),
            capability("send", CapabilityState::Server, "application+mail_server"),
            capability("send_idempotency", CapabilityState::Server, "application+mail_server"),
            capability("drafts", CapabilityState::Server, "application"),
            capability("scheduled_send", CapabilityState::Server, "application+mail_server"),
            capability("contacts", CapabilityState::Server, "application"),
            capability("calendar", CapabilityState::Server, "application"),
            capability("realtime", CapabilityState::Server, "application+mail_server"),
            capability("settings", CapabilityState::Server, "application"),
            capability("billing", CapabilityState::Server, "application"),
            capability("payment_bound_plan_assignment", CapabilityState::Server, "application"),
            capability("localhost_platform_admin", CapabilityState::Server, "application+edge"),
            capability("platform_admin_operations", CapabilityState::Server, "application+edge"),
            capability("platform_runtime_controls", CapabilityState::Server, "application+edge"),
            capability("platform_business_inventory", CapabilityState::Server, "application"),
            capability("platform_hosted_mail_inventory", CapabilityState::Server, "application+mail_server"),
            capability("platform_recovery_center", CapabilityState::Server, "application+mail_server"),
            capability("subscription_lifecycle", CapabilityState::Server, "application"),
            capability("entitlements", CapabilityState::Server, "application+mail_server"),
            capability("deliverability_controls", CapabilityState::Server, "application+mail_server"),
            capability("delivery_event_intake", CapabilityState::Server, "application+mail_server"),
            capability("tenant_suppressions", CapabilityState::Server, "application"),
            capability("remote_image_privacy", CapabilityState::Server, "browser+application"),
            capability("external_mail_clients", CapabilityState::Server, "application+mail_server"),
            capability("mailbox_import", CapabilityState::Server, "application+mail_server"),
            capability("shared_provider_topology", CapabilityState::Server, "application+mail_server+edge"),
            capability("launch_certification", CapabilityState::Server, "application+edge+operator"),
            capability("admin_users", CapabilityState::Server, "application+mail_server"),
            capability("admin_domain", CapabilityState::Server, "application+mail_server"),
            capability("admin_security", CapabilityState::Server, "application+mail_server"),
            capability("admin_forwarders", CapabilityState::Server, "application+mail_server"),
            capability("admin_quarantine", CapabilityState::Server, "application+mail_server"),
            capability("admin_queue", CapabilityState::Server, "mail_server"),
            capability("admin_diagnostics", CapabilityState::Server, "application+mail_server"),
            capability("admin_audit", CapabilityState::Server, "application"),
            capability("aliases", CapabilityState::Server, "application+mail_server"),
            capability("attachments", CapabilityState::Server, "application+mail_server"),
            capability("custom_folders", CapabilityState::Server, "mail_server"),
            capability("labels", CapabilityState::ClientOnly, "browser"),
            capability("identities", CapabilityState::Server, "application+mail_server"),
            capability("linked_accounts", CapabilityState::ClientOnly, "browser"),
            capability("filters", CapabilityState::Server, "application+mail_server"),
            capability("forwarding", CapabilityState::Server, "application+mail_server"),
            capability("vacation", CapabilityState::Server, "application+mail_server"),
            capability("spam_preferences", CapabilityState::ClientOnly, "browser"),
            capability("templates", CapabilityState::ClientOnly, "browser"),
            capability("notifications", CapabilityState::Server, "application"),
            capability("user_audit", CapabilityState::Server, "application"),
            capability("support", CapabilityState::Server, "application+mail_server"),
            capability("public_plans", CapabilityState::Server, "application"),
            capability("public_status", CapabilityState::Server, "application+mail_server"),
            capability("two_factor", CapabilityState::Server, "application"),
            capability("account_sessions", CapabilityState::Server, "application"),
            capability("password_change", CapabilityState::Server, "application+mail_server"),
        ],
    })
}

const fn capability(
    key: &'static str,
    state: CapabilityState,
    authority: &'static str,
) -> Capability {
    Capability {
        key,
        state,
        authority,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn meta_contract_is_versioned_and_unique() {
        let Json(meta) = get().await;
        assert_eq!(meta.contract_version, 32);
        assert_eq!(meta.product, "CS Mail");
        assert_eq!(meta.mail_backend, "self_hosted");

        let mut keys = meta.capabilities.iter().map(|item| item.key).collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), meta.capabilities.len());
    }
}
