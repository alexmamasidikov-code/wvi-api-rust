//! WVI v3 forecast — 24 hourly points ahead, mean + low/high range,
//! plus a short Claude narrative. Cached 15 min per user.

use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
pub struct ForecastPoint {
    pub hour_offset: i32,
    pub mean: f64,
    pub low: f64,
    pub high: f64,
    pub ts: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct WviForecast {
    pub plus_6h: ForecastPoint,
    pub plus_24h: ForecastPoint,
    pub timeline: Vec<ForecastPoint>,
    pub narrative: String,
    pub generated_at: DateTime<Utc>,
}

pub async fn forecast_wvi(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
    current_wvi: f64,
) -> anyhow::Result<WviForecast> {
    // Cache check — 15 min TTL.
    let cached: Option<(
        DateTime<Utc>,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT generated_at, horizon_6h, horizon_24h, timeline, narrative
         FROM wvi_forecast_cache WHERE user_id=$1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    if let Some((gen_at, h6, h24, tl, narr)) = &cached {
        if Utc::now() - *gen_at < Duration::minutes(15) {
            return Ok(WviForecast {
                plus_6h: serde_json::from_value(h6.clone())?,
                plus_24h: serde_json::from_value(h24.clone())?,
                timeline: serde_json::from_value(tl.clone())?,
                narrative: narr.clone().unwrap_or_default(),
                generated_at: *gen_at,
            });
        }
    }

    // Circadian baseline arrays (24-hour expected mean + variance). Defaults
    // keep the forecast flat around 50 with std 5 when nothing is locked.
    let circadian: Vec<(i32, f64, f64)> = sqlx::query_as(
        "SELECT hour_of_day, mean, std FROM circadian_baselines
         WHERE user_id=$1 AND metric_type='wvi' ORDER BY hour_of_day",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut circadian_arr = [50.0_f64; 24];
    let mut variance_arr = [25.0_f64; 24];
    for (h, m, s) in circadian {
        if (0..24).contains(&h) {
            circadian_arr[h as usize] = m;
            variance_arr[h as usize] = s.powi(2);
        }
    }

    // Upcoming events — MVP: empty. Hooked to Project E alarms in integration.
    let upcoming_events: Vec<(DateTime<Utc>, String)> = vec![];

    let mut timeline = Vec::with_capacity(24);
    for h in 0..24 {
        let t = now + Duration::hours(h as i64);
        let hour_of_day = t.hour() as usize;
        let circ_now = circadian_arr[now.hour() as usize];
        let circ_t = circadian_arr[hour_of_day];

        let mut mean = current_wvi + (circ_t - circ_now);

        for (ev_ts, ev_type) in &upcoming_events {
            if *ev_ts > now && *ev_ts <= t {
                mean += match ev_type.as_str() {
                    "workout" => -5.0,
                    "sleep_onset" => 8.0,
                    "meal" => -2.0,
                    _ => 0.0,
                };
            }
        }

        let base_var = variance_arr[hour_of_day];
        let hours_ahead = h as f64;
        let var_total = base_var * (1.0 + hours_ahead.sqrt());
        let std = var_total.sqrt();

        timeline.push(ForecastPoint {
            hour_offset: h,
            mean: mean.clamp(0.0, 100.0),
            low: (mean - std).clamp(0.0, 100.0),
            high: (mean + std).clamp(0.0, 100.0),
            ts: t,
        });
    }

    let plus_6h = timeline.get(6).cloned().unwrap_or_else(|| timeline[0].clone());
    let plus_24h = timeline
        .last()
        .cloned()
        .unwrap_or_else(|| timeline[0].clone());

    // Narrative — call Claude (non-fatal). Uses the Insights endpoint kind for
    // its short fallback copy.
    let narr_prompt = format!(
        "Forecast: now {:.0} → +6h {:.0} → +24h {:.0}. Write 2 sentences in Russian \
         explaining the trajectory. No medical diagnoses. Focus on whether the user \
         should rest, stay active, or watch for drift.",
        current_wvi, plus_6h.mean, plus_24h.mean
    );
    let narrative =
        crate::ai::cli::ask_or_fallback(crate::ai::cli::AiEndpointKind::Insights, &narr_prompt).await;

    sqlx::query(
        "INSERT INTO wvi_forecast_cache (user_id, generated_at, horizon_6h, horizon_24h, timeline, narrative)
         VALUES ($1, NOW(), $2, $3, $4, $5)
         ON CONFLICT (user_id) DO UPDATE SET
           generated_at=NOW(), horizon_6h=$2, horizon_24h=$3, timeline=$4, narrative=$5",
    )
    .bind(user_id)
    .bind(serde_json::to_value(&plus_6h)?)
    .bind(serde_json::to_value(&plus_24h)?)
    .bind(serde_json::to_value(&timeline)?)
    .bind(&narrative)
    .execute(pool)
    .await?;

    Ok(WviForecast {
        plus_6h,
        plus_24h,
        timeline,
        narrative,
        generated_at: Utc::now(),
    })
}
