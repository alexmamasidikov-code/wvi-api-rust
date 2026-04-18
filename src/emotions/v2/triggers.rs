//! Emotion v2 — trigger inference.
//!
//! After each emotion-shift event we look for candidate context events
//! (workouts, BP spikes, stress pulses, etc.) in the preceding 5-minute
//! window and persist a trigger row with a naive correlation-p placeholder.
//! The MVP p is a constant; a follow-up will compute proper base-rate
//! contingency.

use sqlx::PgPool;
use uuid::Uuid;

pub async fn infer(pool: &PgPool, user_id: Uuid) -> sqlx::Result<()> {
    let shifts: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, ts FROM intraday_events
         WHERE user_id=$1 AND event_type='emotion_shift' AND ts > NOW() - INTERVAL '30 minutes'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    for (_shift_id, shift_ts) in shifts {
        let candidates: Vec<(uuid::Uuid, String)> = sqlx::query_as(
            "SELECT id, event_type FROM intraday_events
             WHERE user_id=$1 AND ts BETWEEN $2 AND $3
               AND event_type NOT IN ('emotion_shift','stress_pulse')",
        )
        .bind(user_id)
        .bind(shift_ts - chrono::Duration::minutes(5))
        .bind(shift_ts)
        .fetch_all(pool)
        .await?;

        for (cand_id, _event_type) in candidates {
            let p = 0.2; // MVP constant; real impl: Fisher exact vs base rate.
            if p < 0.3 {
                sqlx::query(
                    "INSERT INTO emotion_triggers
                        (user_id, shift_ts, event_id, correlation_p,
                         shift_from_region, shift_to_region, magnitude)
                     VALUES ($1, $2, $3, $4, 'unknown', 'unknown', 0.5)",
                )
                .bind(user_id)
                .bind(shift_ts)
                .bind(cand_id)
                .bind(p)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}
