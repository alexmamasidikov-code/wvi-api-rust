//! Shared per-user timezone scheduling helpers for daily narrator crons.
//!
//! Each caller (sensitivity, emotions v2, wvi v3) ticks every 5 min, iterates
//! users, and asks `should_fire_morning` / `should_fire_evening` whether the
//! *local* clock is inside the 07:00 / 21:00 five-minute firing window, then
//! calls `record_fire` which atomically inserts into `daily_brief_log` and
//! returns `true` only if no row for that (user, kind) exists yet for the
//! current local date — preventing double-firing on restarts.

use chrono::{DateTime, Datelike, TimeZone, Timelike};
use chrono_tz::Tz;
use sqlx::PgPool;
use uuid::Uuid;

const MORNING_HOUR: u32 = 7;
const EVENING_HOUR: u32 = 21;
const WINDOW_MINUTES: u32 = 5;

pub fn should_fire_morning(now_local: &DateTime<Tz>) -> bool {
    now_local.hour() == MORNING_HOUR && now_local.minute() < WINDOW_MINUTES
}

pub fn should_fire_evening(now_local: &DateTime<Tz>) -> bool {
    now_local.hour() == EVENING_HOUR && now_local.minute() < WINDOW_MINUTES
}

/// Atomically claim a firing slot. Returns `Ok(true)` if this call won the
/// race and should fire the brief, `Ok(false)` if another tick already did.
/// The key is (user_id, kind, start-of-local-day converted to UTC) so each
/// local day gets exactly one slot per kind.
pub async fn record_fire(
    pool: &PgPool,
    user_id: Uuid,
    kind: &str,
    now_local: &DateTime<Tz>,
) -> sqlx::Result<bool> {
    let tz = now_local.timezone();
    let start_of_day = tz
        .with_ymd_and_hms(now_local.year(), now_local.month(), now_local.day(), 0, 0, 0)
        .single()
        .unwrap_or_else(|| tz.from_utc_datetime(&now_local.naive_utc()))
        .with_timezone(&chrono::Utc);

    let affected = sqlx::query(
        "INSERT INTO daily_brief_log (user_id, kind, fired_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (user_id, kind, fired_at) DO NOTHING",
    )
    .bind(user_id)
    .bind(kind)
    .bind(start_of_day)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use chrono_tz::America::New_York;

    #[test]
    fn fires_at_7_00_local() {
        let t = New_York.with_ymd_and_hms(2026, 4, 18, 7, 2, 0).unwrap();
        assert!(should_fire_morning(&t));
        assert!(!should_fire_evening(&t));
    }

    #[test]
    fn no_fire_at_7_06_local() {
        let t = New_York.with_ymd_and_hms(2026, 4, 18, 7, 6, 0).unwrap();
        assert!(!should_fire_morning(&t));
    }

    #[test]
    fn fires_at_21_00_local() {
        let t = New_York.with_ymd_and_hms(2026, 4, 18, 21, 0, 0).unwrap();
        assert!(should_fire_evening(&t));
    }

    #[test]
    fn ny_7am_is_not_utc_7am() {
        // 07:00 EDT = 11:00 UTC — proof the gate is per-local-tz.
        let utc_11 = Utc.with_ymd_and_hms(2026, 4, 18, 11, 0, 0).unwrap();
        let ny_local = utc_11.with_timezone(&New_York);
        assert!(should_fire_morning(&ny_local));
        let utc_7 = Utc.with_ymd_and_hms(2026, 4, 18, 7, 0, 0).unwrap();
        let ny_local_utc7 = utc_7.with_timezone(&New_York);
        assert!(!should_fire_morning(&ny_local_utc7));
    }
}
