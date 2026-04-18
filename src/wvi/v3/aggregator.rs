//! WVI v3 aggregator.
//!
//! `compute_wvi_v3` is the one public entry point called by handlers.
//! It loads the user's profile + base weights, applies runtime context,
//! pulls the 18 component scores (now wired to real biometric + baseline
//! + emotion data), blends a small AI nudge, then returns score + tier
//! + pillar breakdown + weights.

use chrono::{DateTime, Timelike, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::sensitivity::baseline;
use crate::sensitivity::types::{ActivityState, Baseline, ContextKey};

#[derive(Serialize)]
pub struct WviV3Result {
    pub current: f64,
    pub tier: String,
    pub momentum: f64,
    pub volatility: f64,
    pub pillars: HashMap<String, f64>,
    pub components: HashMap<String, f64>,
    pub weights: HashMap<String, f64>,
    pub ai_blend_factor: f64,
    pub ai_rationale: Option<String>,
    pub formula_version: i32,
    pub computed_at: DateTime<Utc>,
}

pub async fn compute_wvi_v3(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> anyhow::Result<WviV3Result> {
    // 1. Load profile.
    let profile_row: Option<(String,)> =
        sqlx::query_as("SELECT profile FROM user_wvi_profile WHERE user_id=$1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let profile = profile_row
        .map(|(p,)| p)
        .unwrap_or_else(|| "balanced".to_string());

    // 2. Load base weights.
    let mut weights = crate::wvi::v3::reweighting::load_base_weights(pool, &profile).await?;
    if weights.is_empty() {
        // Fall back to balanced when a user's profile row references a profile
        // without seeded weights (shouldn't happen once 014 runs).
        weights = crate::wvi::v3::reweighting::load_base_weights(pool, "balanced").await?;
    }

    // 3. Apply runtime context.
    let ctx = determine_context(pool, user_id, now).await?;
    crate::wvi::v3::reweighting::apply_context(&mut weights, &ctx);

    // 4. Compute components (MVP: 50.0 placeholder per plan — full wiring
    //    pending per-component baseline queries).
    let components = compute_all_components(pool, user_id, now).await?;

    // 5. Weighted sum.
    let weighted: f64 = components
        .iter()
        .map(|(k, v)| weights.get(k).copied().unwrap_or(0.0) * v)
        .sum();

    // 6. AI blend factor (MVP stub: 0 blend, no rationale).
    let (ai_blend, ai_rationale) = get_ai_blend(pool, user_id, &components, &ctx).await?;

    let wvi_current = (0.9 * weighted + 0.1 * (50.0 + ai_blend * 10.0)).clamp(0.0, 100.0);

    // 7. Momentum + volatility.
    let momentum = compute_momentum(pool, user_id).await.unwrap_or(0.0);
    let volatility = compute_volatility(pool, user_id).await.unwrap_or(0.0);

    // 8. Pillar scores (weighted mean of member components).
    let pillars = compute_pillars(&components, &weights);

    let tier = classify_tier(wvi_current);

    Ok(WviV3Result {
        current: wvi_current,
        tier,
        momentum,
        volatility,
        pillars,
        components,
        weights,
        ai_blend_factor: ai_blend,
        ai_rationale,
        formula_version: 3,
        computed_at: now,
    })
}

fn classify_tier(score: f64) -> String {
    let t = match score {
        s if s < 20.0 => "Critical",
        s if s < 40.0 => "Low",
        s if s < 55.0 => "Moderate",
        s if s < 70.0 => "Good",
        s if s < 85.0 => "Excellent",
        s if s < 95.0 => "Super",
        _ => "Perfect",
    };
    t.to_string()
}

fn compute_pillars(
    comps: &HashMap<String, f64>,
    weights: &HashMap<String, f64>,
) -> HashMap<String, f64> {
    let pillar_map: Vec<(&str, Vec<&str>)> = vec![
        ("Recovery", vec!["hrv_personal", "signal_burden", "recovery_momentum"]),
        ("Sleep", vec!["sleep_composite", "circadian_alignment"]),
        ("Activity", vec!["activity_personal", "metabolic_efficiency"]),
        (
            "Stress",
            vec!["stress_personal", "breathing_rate_rest", "coherence_personal"],
        ),
        ("Resilience", vec!["immune_proxy", "intraday_stability"]),
        (
            "Emotional",
            vec![
                "emotion_agility",
                "emotion_range",
                "emotion_anchors",
                "emotion_regulation",
                "emotion_diversity",
                "emotion_contagion",
            ],
        ),
    ];

    let mut out: HashMap<String, f64> = HashMap::new();
    for (pillar, members) in pillar_map {
        let (sum_w, sum_wv): (f64, f64) = members.iter().fold((0.0, 0.0), |(sw, swv), k| {
            let w = *weights.get(*k).unwrap_or(&0.0);
            let v = *comps.get(*k).unwrap_or(&50.0);
            (sw + w, swv + w * v)
        });
        out.insert(
            pillar.to_string(),
            if sum_w > 0.0 { sum_wv / sum_w } else { 50.0 },
        );
    }
    out
}

async fn determine_context(
    _pool: &PgPool,
    _user_id: Uuid,
    now: DateTime<Utc>,
) -> anyhow::Result<crate::wvi::v3::reweighting::RuntimeContext> {
    let hour = now.hour();
    Ok(crate::wvi::v3::reweighting::RuntimeContext {
        in_sleep_window: hour < 7 || hour >= 23,
        post_workout_minutes: -1,
        in_active_hours: (9..22).contains(&hour),
        illness_mode: false,
    })
}

/// Compute all 18 component scores by querying real data sources and
/// dispatching to the pure component functions in `components.rs`. Each
/// branch degrades to `50.0` on missing data rather than failing.
async fn compute_all_components(
    pool: &PgPool,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> anyhow::Result<HashMap<String, f64>> {
    use super::components as comp;

    let ctx = ContextKey::from_ts(now, ActivityState::Resting);
    let mut out: HashMap<String, f64> = HashMap::new();

    // ---- Recovery pillar -----------------------------------------------
    let hrv_now = latest_value(pool, user_id, "hrv").await.unwrap_or(50.0);
    let hrv_bl = baseline::load(pool, user_id, "hrv", &ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(default_baseline);
    out.insert("hrv_personal".into(), comp::hrv_personal_score(hrv_now, &hrv_bl));

    let signal_penalty = signal_burden_total(pool, user_id).await.unwrap_or(0.0);
    out.insert("signal_burden".into(), comp::signal_burden(signal_penalty));

    let (avg_7d, avg_30d) = wvi_rolling_avgs(pool, user_id).await;
    out.insert("recovery_momentum".into(), comp::recovery_momentum(avg_7d, avg_30d));

    // ---- Sleep pillar --------------------------------------------------
    out.insert(
        "sleep_composite".into(),
        sleep_composite_from_latest(pool, user_id, &ctx)
            .await
            .unwrap_or(50.0),
    );
    out.insert(
        "circadian_alignment".into(),
        circadian_alignment_last_24h(pool, user_id)
            .await
            .unwrap_or(50.0),
    );

    // ---- Activity pillar ----------------------------------------------
    out.insert(
        "activity_personal".into(),
        activity_personal(pool, user_id, &ctx).await.unwrap_or(50.0),
    );
    out.insert(
        "metabolic_efficiency".into(),
        metabolic_efficiency_from_hr(pool, user_id).await.unwrap_or(50.0),
    );

    // ---- Stress pillar ------------------------------------------------
    let stress_now = latest_value(pool, user_id, "stress").await.unwrap_or(50.0);
    let stress_bl = baseline::load(pool, user_id, "stress", &ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(default_baseline);
    out.insert("stress_personal".into(), comp::stress_personal(stress_now, &stress_bl));

    let br_now = latest_value(pool, user_id, "breathing_rate")
        .await
        .unwrap_or(16.0);
    let br_bl = baseline::load(pool, user_id, "breathing_rate", &ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| Baseline {
            mean: 16.0,
            std: 2.0,
            p10: 14.0,
            p90: 18.0,
            sample_count: 0,
            locked: false,
        });
    out.insert("breathing_rate_rest".into(), comp::breathing_rate_rest(br_now, &br_bl));

    let coh_now = latest_value(pool, user_id, "coherence").await.unwrap_or(0.5);
    let coh_bl = baseline::load(pool, user_id, "coherence", &ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| Baseline {
            mean: 0.5,
            std: 0.15,
            p10: 0.35,
            p90: 0.65,
            sample_count: 0,
            locked: false,
        });
    out.insert(
        "coherence_personal".into(),
        comp::coherence_personal(coh_now, &coh_bl),
    );

    // ---- Resilience pillar --------------------------------------------
    out.insert(
        "immune_proxy".into(),
        immune_proxy_compute(pool, user_id).await.unwrap_or(50.0),
    );
    out.insert(
        "intraday_stability".into(),
        intraday_stability_24h(pool, user_id).await.unwrap_or(50.0),
    );

    // ---- Emotional pillar ---------------------------------------------
    if let Ok(metrics) = crate::emotions::v2::metrics::compute(pool, user_id).await {
        out.insert("emotion_agility".into(), metrics.agility);
        out.insert("emotion_range".into(), metrics.range);
        let dwell: Vec<f64> = metrics.anchors.iter().map(|(_, p)| *p).collect();
        out.insert("emotion_anchors".into(), comp::emotion_anchors(&dwell));
        out.insert("emotion_regulation".into(), metrics.regulation);
        out.insert("emotion_diversity".into(), metrics.diversity);
        out.insert("emotion_contagion".into(), metrics.contagion);
    } else {
        for c in [
            "emotion_agility",
            "emotion_range",
            "emotion_anchors",
            "emotion_regulation",
            "emotion_diversity",
            "emotion_contagion",
        ] {
            out.insert(c.into(), 50.0);
        }
    }

    Ok(out)
}

fn default_baseline() -> Baseline {
    Baseline {
        mean: 50.0,
        std: 10.0,
        p10: 40.0,
        p90: 60.0,
        sample_count: 0,
        locked: false,
    }
}

// ── Helper queries ───────────────────────────────────────────────────────

async fn latest_value(pool: &PgPool, user_id: Uuid, metric: &str) -> Option<f64> {
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT value FROM biometrics_1min
         WHERE user_id=$1 AND metric_type=$2
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(metric)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    row.map(|(v,)| v)
}

/// Map unacknowledged signals of the last 24h to a penalty total, where
/// each signal contributes per severity: low=5, medium=12, high=25.
async fn signal_burden_total(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT severity FROM signals
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours' AND NOT ack",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let mut total: f64 = 0.0;
    for (sev,) in rows {
        total += match sev.as_str() {
            "low" => 5.0_f64,
            "medium" => 12.0,
            "high" => 25.0,
            _ => 0.0,
        };
    }
    Ok(total.min(100.0))
}

async fn wvi_rolling_avgs(pool: &PgPool, user_id: Uuid) -> (f64, f64) {
    let r7: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(value_mean) FROM biometrics_daily
         WHERE user_id=$1 AND metric_type='wvi' AND day > CURRENT_DATE - 7",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let r30: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(value_mean) FROM biometrics_daily
         WHERE user_id=$1 AND metric_type='wvi' AND day > CURRENT_DATE - 30",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let a7 = r7.and_then(|(v,)| v).unwrap_or(50.0);
    let a30 = r30.and_then(|(v,)| v).unwrap_or(50.0);
    (a7, a30)
}

async fn sleep_composite_from_latest(
    pool: &PgPool,
    user_id: Uuid,
    ctx: &ContextKey,
) -> sqlx::Result<f64> {
    use super::components as comp;
    let row: Option<(Option<f32>, Option<f32>, Option<f32>)> = sqlx::query_as(
        "SELECT total_hours, deep_percent, efficiency
         FROM sleep_records
         WHERE user_id=$1
         ORDER BY date DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let (hours, deep_pct, efficiency) = match row {
        Some((h, d, e)) => (
            h.unwrap_or(7.5) as f64,
            d.unwrap_or(20.0) as f64,
            e.unwrap_or(85.0) as f64,
        ),
        None => return Ok(50.0),
    };
    let minutes = hours * 60.0;
    let continuity = (efficiency / 100.0).clamp(0.0, 1.0);

    let bl = baseline::load(pool, user_id, "sleep_score", ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| Baseline {
            mean: 70.0,
            std: 10.0,
            p10: 60.0,
            p90: 80.0,
            sample_count: 0,
            locked: false,
        });
    Ok(comp::sleep_composite(minutes, deep_pct, continuity, &bl))
}

async fn circadian_alignment_last_24h(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    use super::components as comp;

    let recent: Vec<(i32, f64)> = sqlx::query_as(
        "SELECT EXTRACT(HOUR FROM ts)::INT as h, AVG(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='hr' AND ts > NOW() - INTERVAL '24 hours'
         GROUP BY h ORDER BY h",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut recent_24h = vec![0.0f64; 24];
    for (h, v) in recent {
        let idx = (h.rem_euclid(24)) as usize;
        recent_24h[idx] = v;
    }

    let baseline_rows: Vec<(i32, f64)> = sqlx::query_as(
        "SELECT hour_of_day, mean FROM circadian_baselines
         WHERE user_id=$1 AND metric_type='hr'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    if baseline_rows.is_empty() {
        return Ok(50.0);
    }
    let mut baseline = [0.0f64; 24];
    for (h, v) in baseline_rows {
        let idx = (h.rem_euclid(24)) as usize;
        baseline[idx] = v;
    }
    Ok(comp::circadian_alignment(&recent_24h, &baseline))
}

async fn activity_personal(
    pool: &PgPool,
    user_id: Uuid,
    ctx: &ContextKey,
) -> sqlx::Result<f64> {
    use super::components as comp;
    let steps_row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT SUM(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='steps' AND ts::date = CURRENT_DATE",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let active_row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='activity_intensity'
           AND ts::date = CURRENT_DATE AND value > 10.0",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let steps = steps_row.and_then(|(v,)| v).unwrap_or(0.0);
    let active_mins = active_row.and_then(|(v,)| v).unwrap_or(0) as f64;

    let bl = baseline::load(pool, user_id, "steps", ctx)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| Baseline {
            mean: 7000.0,
            std: 3000.0,
            p10: 4000.0,
            p90: 10000.0,
            sample_count: 0,
            locked: false,
        });
    Ok(comp::activity_personal_score(steps, active_mins, &bl))
}

async fn metabolic_efficiency_from_hr(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    use super::components as comp;
    // Active window = last 7 days with intensity > 10; rest window = sleeping hours.
    let act_row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(b.value) FROM biometrics_1min b
         JOIN biometrics_1min i
           ON i.user_id=b.user_id AND i.ts=b.ts AND i.metric_type='activity_intensity'
         WHERE b.user_id=$1 AND b.metric_type='hr'
           AND b.ts > NOW() - INTERVAL '7 days'
           AND i.value > 10",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let rest_row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='hr'
           AND ts > NOW() - INTERVAL '7 days'
           AND EXTRACT(HOUR FROM ts) BETWEEN 1 AND 5",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let avg_active = act_row.and_then(|(v,)| v).unwrap_or(0.0);
    let avg_rest = rest_row.and_then(|(v,)| v).unwrap_or(60.0);
    if avg_active < 1e-6 || avg_rest < 1e-6 {
        return Ok(50.0);
    }
    // Recovery time proxy — no ECG recovery series yet; use 90s default.
    let recovery_sec = 90.0;
    Ok(comp::metabolic_efficiency(avg_active, avg_rest, recovery_sec))
}

async fn immune_proxy_compute(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    use super::components as comp;
    // Temperature stability — inverse of sd over last 7 days mapped into 0..100.
    let temp_row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT STDDEV_POP(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='temperature'
           AND ts > NOW() - INTERVAL '7 days'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let temp_sd = temp_row.and_then(|(v,)| v).unwrap_or(0.3);
    let temp_stab = (100.0 - (temp_sd * 100.0)).clamp(0.0, 100.0);

    // Sleep quality — 7-day mean of `sleep_score`.
    let sleep_row: Option<(Option<f32>,)> = sqlx::query_as(
        "SELECT AVG(sleep_score) FROM sleep_records
         WHERE user_id=$1 AND date > CURRENT_DATE - 7",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let sleep_q = sleep_row.and_then(|(v,)| v).unwrap_or(70.0) as f64;

    // HRV stability — lower cv ⇒ higher score.
    let hrv_row: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT AVG(value), STDDEV_POP(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='hrv'
           AND ts > NOW() - INTERVAL '7 days'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let hrv_stab = match hrv_row {
        Some((Some(mean), Some(sd))) if mean > 1.0 => {
            let cv = sd / mean;
            (100.0 - cv * 200.0).clamp(0.0, 100.0)
        }
        _ => 60.0,
    };

    Ok(comp::immune_proxy(temp_stab, sleep_q, hrv_stab))
}

async fn intraday_stability_24h(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    use super::components as comp;
    let rows: Vec<(f64,)> = sqlx::query_as(
        "SELECT value FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='wvi' AND ts > NOW() - INTERVAL '24 hours'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let vals: Vec<f64> = rows.into_iter().map(|(v,)| v).collect();
    Ok(comp::intraday_stability(&vals))
}

async fn get_ai_blend(
    _pool: &PgPool,
    _user_id: Uuid,
    _comps: &HashMap<String, f64>,
    _ctx: &crate::wvi::v3::reweighting::RuntimeContext,
) -> anyhow::Result<(f64, Option<String>)> {
    Ok((0.0, None))
}

async fn compute_momentum(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    let row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(value_mean) FROM biometrics_5min
         WHERE user_id=$1 AND metric_type='wvi' AND bucket_ts > NOW() - INTERVAL '7 days'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| v).unwrap_or(50.0) - 50.0)
}

async fn compute_volatility(pool: &PgPool, user_id: Uuid) -> sqlx::Result<f64> {
    let row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT STDDEV_POP(value) FROM biometrics_1min
         WHERE user_id=$1 AND metric_type='wvi' AND ts > NOW() - INTERVAL '24 hours'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| v).unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_boundaries() {
        assert_eq!(classify_tier(10.0), "Critical");
        assert_eq!(classify_tier(30.0), "Low");
        assert_eq!(classify_tier(50.0), "Moderate");
        assert_eq!(classify_tier(65.0), "Good");
        assert_eq!(classify_tier(80.0), "Excellent");
        assert_eq!(classify_tier(90.0), "Super");
        assert_eq!(classify_tier(99.0), "Perfect");
    }

    #[test]
    fn pillars_reflect_components() {
        let mut comps = HashMap::new();
        for c in [
            "hrv_personal",
            "signal_burden",
            "recovery_momentum",
            "sleep_composite",
            "circadian_alignment",
            "activity_personal",
            "metabolic_efficiency",
            "stress_personal",
            "breathing_rate_rest",
            "coherence_personal",
            "immune_proxy",
            "intraday_stability",
            "emotion_agility",
            "emotion_range",
            "emotion_anchors",
            "emotion_regulation",
            "emotion_diversity",
            "emotion_contagion",
        ] {
            comps.insert(c.to_string(), 75.0);
        }
        let mut w: HashMap<String, f64> = comps.keys().map(|k| (k.clone(), 0.1)).collect();
        crate::wvi::v3::reweighting::renormalize(&mut w);
        let pillars = compute_pillars(&comps, &w);
        assert!((pillars["Recovery"] - 75.0).abs() < 0.01);
        assert!((pillars["Sleep"] - 75.0).abs() < 0.01);
        assert_eq!(pillars.len(), 6);
    }
}
