use axum::{extract::State, Json};
use serde::Serialize;
use sqlx::PgPool;

use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// Single celebratory micro-fact computed for the HOME "TODAY'S WIN" card.
///
/// The contract matches what the iOS client decoded from a placeholder
/// heuristic in `TodayWinCard.swift` so we can swap the data source without
/// touching the UI: rank candidate "wins" by how positive each is for the
/// user *today vs their personal baseline*, return the strongest one.
///
/// All deltas are computed against the user's own 14-day rolling average
/// (Personal-baseline approach beats absolute thresholds because "good HRV
/// for me" lives in a different range than the same number for someone else).
#[derive(Serialize)]
pub struct TodayWin {
    pub metric: String,
    pub narrative: String,
    pub icon: String,
    #[serde(rename = "colorHex")]
    pub color_hex: String,
    pub delta: f64,
}

/// `GET /api/v1/insights/daily-win`
///
/// Always returns 200 with a `TodayWin` payload — the card is the daily
/// return-hook on HOME, an empty response would leave the user staring at
/// a hole. When no positive delta exists the handler falls through to a
/// soft encouragement so the card always has something honest to say.
pub async fn daily_win(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<TodayWin>> {
    let user_id_row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM users WHERE privy_did = $1",
    )
    .bind(&user.privy_did)
    .fetch_optional(&pool)
    .await?;

    let Some((user_id,)) = user_id_row else {
        return Ok(Json(starter_win()));
    };

    // Today's HRV: max RMSSD recorded in the last 24 h (matches the
    // "best of the day" framing we want for the celebratory card).
    let hrv_today: Option<f64> = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT MAX(value)::float8 FROM hrv \
         WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| r.0);

    // Rolling 14-day baseline excluding today (so today's value can stand
    // out against itself's own average).
    let hrv_baseline: Option<f64> = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(daily_max)::float8 FROM ( \
            SELECT date_trunc('day', timestamp) AS d, MAX(value) AS daily_max \
            FROM hrv \
            WHERE user_id = $1 \
              AND timestamp >= NOW() - INTERVAL '14 days' \
              AND timestamp < date_trunc('day', NOW()) \
            GROUP BY 1 \
         ) t",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| r.0);

    let sleep_today: Option<f64> = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT sleep_score::float8 FROM sleep_records \
         WHERE user_id = $1 ORDER BY date DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| r.0);

    let sleep_baseline: Option<f64> = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(sleep_score)::float8 FROM sleep_records \
         WHERE user_id = $1 \
           AND date >= CURRENT_DATE - INTERVAL '14 days' \
           AND date < CURRENT_DATE",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| r.0);

    let steps_today: Option<f64> = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT SUM(steps)::float8 FROM activity \
         WHERE user_id = $1 AND timestamp >= CURRENT_DATE",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| r.0);

    let wvi_today: Option<(f64, f64)> = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "WITH t AS ( \
            SELECT wvi_score::float8 AS today_score \
            FROM wvi_scores \
            WHERE user_id = $1 AND timestamp >= CURRENT_DATE \
            ORDER BY timestamp DESC LIMIT 1 \
         ), y AS ( \
            SELECT wvi_score::float8 AS yesterday_score \
            FROM wvi_scores \
            WHERE user_id = $1 \
              AND timestamp >= CURRENT_DATE - INTERVAL '1 day' \
              AND timestamp < CURRENT_DATE \
            ORDER BY timestamp DESC LIMIT 1 \
         ) \
         SELECT (SELECT today_score FROM t), (SELECT yesterday_score FROM y)",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .ok()
    .and_then(|r| match (r.0, r.1) {
        (Some(t), Some(y)) => Some((t, y)),
        _ => None,
    });

    // Rank candidates by relative improvement. Each block returns Option<TodayWin>;
    // the first non-None in priority order wins.

    if let Some((today, yesterday)) = wvi_today {
        let delta = today - yesterday;
        if delta >= 3.0 {
            return Ok(Json(TodayWin {
                metric: "WVI".into(),
                narrative: format!(
                    "WVI is up +{} today — your habits are compounding.",
                    delta as i32
                ),
                icon: "sparkles".into(),
                color_hex: "#50E88F".into(),
                delta,
            }));
        }
    }

    if let (Some(today), Some(baseline)) = (hrv_today, hrv_baseline) {
        if baseline > 0.0 {
            let delta = (today - baseline) / baseline;
            if delta >= 0.10 {
                return Ok(Json(TodayWin {
                    metric: "HRV".into(),
                    narrative: format!(
                        "HRV {} ms — {}% above your two-week average.",
                        today as i32,
                        (delta * 100.0) as i32
                    ),
                    icon: "heart.text.square".into(),
                    color_hex: "#50E88F".into(),
                    delta,
                }));
            }
        }
    }

    if let (Some(today), Some(baseline)) = (sleep_today, sleep_baseline) {
        if baseline > 0.0 {
            let delta = (today - baseline) / baseline;
            if delta >= 0.05 {
                return Ok(Json(TodayWin {
                    metric: "SLEEP".into(),
                    narrative: format!(
                        "Sleep score {} — best in your recent week.",
                        today as i32
                    ),
                    icon: "moon.stars".into(),
                    color_hex: "#8B9DFF".into(),
                    delta,
                }));
            }
        }
    } else if let Some(today) = sleep_today {
        if today >= 70.0 {
            return Ok(Json(TodayWin {
                metric: "SLEEP".into(),
                narrative: format!("Sleep score {} — solid night of recovery.", today as i32),
                icon: "moon.stars".into(),
                color_hex: "#8B9DFF".into(),
                delta: 0.0,
            }));
        }
    }

    if let Some(steps) = steps_today {
        if steps >= 5_000.0 {
            return Ok(Json(TodayWin {
                metric: "STEPS".into(),
                narrative: format!(
                    "{} steps already today — keep the rhythm going.",
                    format_thousands(steps as i64)
                ),
                icon: "figure.walk".into(),
                color_hex: "#FCC73D".into(),
                delta: 0.0,
            }));
        }
    }

    // Soft fallback — always return something so the card stays visible.
    if let Some(today) = hrv_today {
        return Ok(Json(TodayWin {
            metric: "HRV".into(),
            narrative: format!(
                "HRV {} ms — your nervous system data for today is online.",
                today as i32
            ),
            icon: "waveform.path.ecg".into(),
            color_hex: "#7B6CE7".into(),
            delta: 0.0,
        }));
    }

    if let Some(steps) = steps_today {
        if steps > 100.0 {
            return Ok(Json(TodayWin {
                metric: "MOVE".into(),
                narrative: format!("{} steps so far — every step counts.", format_thousands(steps as i64)),
                icon: "figure.walk".into(),
                color_hex: "#FCC73D".into(),
                delta: 0.0,
            }));
        }
    }

    Ok(Json(starter_win()))
}

fn starter_win() -> TodayWin {
    TodayWin {
        metric: "FRESH".into(),
        narrative: "Fresh start. Your bracelet is collecting your baseline.".into(),
        icon: "sun.max".into(),
        color_hex: "#FCC73D".into(),
        delta: 0.0,
    }
}

fn format_thousands(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}
