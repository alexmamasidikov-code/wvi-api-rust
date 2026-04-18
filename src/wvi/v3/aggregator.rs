//! WVI v3 aggregator.
//!
//! `compute_wvi_v3` is the one public entry point called by handlers.
//! It loads the user's profile + base weights, applies runtime context,
//! pulls the 18 component scores (MVP stub — all 50.0 until each component
//! is wired to its actual data source), blends a small AI nudge, then
//! returns score + tier + pillar breakdown + weights.

use chrono::{DateTime, Timelike, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

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

/// MVP stub: all 18 components return 50.0. Full implementation will query
/// baselines + biometric 5-min aggregates and dispatch to each component fn.
async fn compute_all_components(
    _pool: &PgPool,
    _user_id: Uuid,
    _now: DateTime<Utc>,
) -> anyhow::Result<HashMap<String, f64>> {
    let mut out = HashMap::new();
    for comp in [
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
        out.insert(comp.to_string(), 50.0);
    }
    Ok(out)
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
