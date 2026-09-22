use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

/// Best-effort write to the audit trail. Failures are logged, never fatal.
pub async fn record(state: &AppState, actor_id: Option<Uuid>, action: &str, detail: Value) {
    let result =
        sqlx::query("INSERT INTO audit_log (actor_id, action, detail) VALUES ($1, $2, $3)")
            .bind(actor_id)
            .bind(action)
            .bind(&detail)
            .execute(&state.db)
            .await;
    match result {
        Ok(_) => {
            if let Some(user_id) = actor_id {
                crate::services::notifications::from_audit_action(state, user_id, action, &detail).await;
            }
        }
        Err(e) => tracing::warn!("audit log write failed for {action}: {e}"),
    }
}
