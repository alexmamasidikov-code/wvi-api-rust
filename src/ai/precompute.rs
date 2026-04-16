//! Background AI pre-generation.
//!
//! Spawned at startup. Every ~5 min, finds users with recent biometric
//! activity and pre-generates the five "panel" AI responses (daily_brief,
//! recovery_deep, full_analysis, ecg_interpret, weekly_deep) into AppCache.
//!
//! Why: the Claude CLI takes 20-40 s for the medical-analyst prompt. Users
//! tapping a panel card should see the answer instantly, not wait. Cache
//! TTL is 10 min (see `AppCache::get_ai`), so a 5 min refresh cadence
//! guarantees cards are always warm while the user is active.
//!
//! Scope: only users seen in the last 15 min of biometric uploads. Idle
//! users don't burn CLI cycles. Panels are generated serially per-user
//! but one kind at a time — the CLI semaphore (max 5 concurrent) already
//! bounds parallelism across the process.

use sqlx::PgPool;
use std::time::Duration;

use crate::cache::AppCache;

use super::cli::AiEndpointKind;
use super::handlers::{
    call_claude, cache_key, DAILY_BRIEF_PROMPT, ECG_INTERPRET_PROMPT, EVENING_REVIEW_PROMPT,
    FULL_ANALYSIS_PROMPT, RECOVERY_DEEP_PROMPT, WEEKLY_DEEP_PROMPT,
};

/// Panels we keep warm. Chat/anomaly alerts are skipped — chat is per-message,
/// anomaly alerts need trigger params at call time.
const PANELS: &[(AiEndpointKind, &str)] = &[
    (AiEndpointKind::DailyBrief, DAILY_BRIEF_PROMPT),
    (AiEndpointKind::RecoveryDeep, RECOVERY_DEEP_PROMPT),
    (AiEndpointKind::FullAnalysis, FULL_ANALYSIS_PROMPT),
    (AiEndpointKind::EveningReview, EVENING_REVIEW_PROMPT),
    (AiEndpointKind::WeeklyDeep, WEEKLY_DEEP_PROMPT),
    (AiEndpointKind::EcgInterpret, ECG_INTERPRET_PROMPT),
];

/// Refresh interval. 5 min balances freshness (new biometrics show up
/// within a cache cycle) against CLI cost (~30 s × 6 panels × active users).
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Window for "active user" detection. Anyone who uploaded biometrics in
/// the last 15 min gets their panels prewarmed.
const ACTIVE_WINDOW: &str = "15 minutes";

/// Spawn the prewarmer. Safe to call once at startup after pool + cache
/// are ready. Returns immediately — work happens on a tokio background task.
pub fn spawn_prewarmer(pool: PgPool, cache: AppCache) {
    tokio::spawn(async move {
        // One tick on startup so the first active user doesn't have to wait
        // for a full 5 min cycle before any cache exists.
        loop {
            if let Err(e) = prewarm_once(&pool, &cache).await {
                tracing::warn!(error = %e, "AI prewarmer tick failed (will retry)");
            }
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

async fn prewarm_once(pool: &PgPool, cache: &AppCache) -> Result<(), sqlx::Error> {
    let users = active_users(pool).await?;
    if users.is_empty() {
        tracing::debug!("AI prewarmer: no active users this tick");
        return Ok(());
    }
    tracing::info!(
        count = users.len(),
        "AI prewarmer: refreshing panels for active users"
    );

    for privy_did in users {
        for (kind, prompt) in PANELS {
            // Generate even if there's an existing cached value — the point
            // of the prewarmer is to keep the response current relative to
            // fresh biometrics. Write overrides the previous entry.
            let text = call_claude(pool, &privy_did, *kind, prompt).await;
            cache.set_ai(&cache_key(&privy_did, *kind), text).await;
        }
    }
    Ok(())
}

/// Users who posted any heart-rate / hrv / spo2 sample in the last window.
async fn active_users(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(&format!(
        r#"
        SELECT DISTINCT u.privy_did
        FROM users u
        WHERE u.id IN (
            SELECT user_id FROM heart_rate
            WHERE timestamp > NOW() - INTERVAL '{window}'
            UNION
            SELECT user_id FROM hrv
            WHERE timestamp > NOW() - INTERVAL '{window}'
            UNION
            SELECT user_id FROM spo2
            WHERE timestamp > NOW() - INTERVAL '{window}'
        )
        "#,
        window = ACTIVE_WINDOW
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}
