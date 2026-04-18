pub mod handlers;

use sqlx::PgPool;
use uuid::Uuid;

/// Resolve internal user UUID from a Privy DID. Shared helper used by feature
/// modules that need the canonical user_id (e.g. intraday ingest, sensitivity,
/// alarms, reminders, WVI v3). Prefer this over inlined subqueries to keep
/// query plans cached and error handling uniform.
pub async fn resolve_user_id(pool: &PgPool, privy_did: &str) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE privy_did = $1")
        .bind(privy_did)
        .fetch_one(pool)
        .await
}
