//! WVI v3 — weight reweighting: load base profile, apply runtime context,
//! optionally blend an AI override (capped, rate-limited).

use chrono::Utc;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct RuntimeContext {
    pub in_sleep_window: bool,
    /// Minutes since most recent workout end. `-1` when no recent workout.
    pub post_workout_minutes: i32,
    pub in_active_hours: bool,
    pub illness_mode: bool,
}

pub async fn load_base_weights(
    pool: &PgPool,
    profile: &str,
) -> sqlx::Result<HashMap<String, f64>> {
    let rows: Vec<(String, f64)> =
        sqlx::query_as("SELECT component, weight FROM profile_component_weights WHERE profile=$1")
            .bind(profile)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub fn apply_context(weights: &mut HashMap<String, f64>, ctx: &RuntimeContext) {
    if ctx.in_sleep_window {
        if let Some(w) = weights.get_mut("sleep_composite") {
            *w *= 2.0;
        }
        if let Some(w) = weights.get_mut("activity_personal") {
            *w *= 0.3;
        }
    }
    if (0..=120).contains(&ctx.post_workout_minutes) {
        if let Some(w) = weights.get_mut("recovery_momentum") {
            *w *= 1.5;
        }
        if let Some(w) = weights.get_mut("hrv_personal") {
            *w *= 1.3;
        }
    }
    if ctx.in_active_hours && !ctx.in_sleep_window {
        if let Some(w) = weights.get_mut("activity_personal") {
            *w *= 1.2;
        }
        if let Some(w) = weights.get_mut("sleep_composite") {
            *w *= 0.5;
        }
    }
    if ctx.illness_mode {
        if let Some(w) = weights.get_mut("immune_proxy") {
            *w *= 2.0;
        }
        if let Some(w) = weights.get_mut("sleep_composite") {
            *w *= 1.5;
        }
    }
    renormalize(weights);
}

pub fn renormalize(weights: &mut HashMap<String, f64>) {
    let sum: f64 = weights.values().sum();
    if sum < 1e-9 {
        return;
    }
    for v in weights.values_mut() {
        *v /= sum;
    }
}

/// Optional AI override — blends up to ±0.1 per-component deltas from Claude.
/// Rate-limited: at most 1 new call every 4h, max 3 uses/day. Returns the
/// rationale string (Russian, 1 sentence) when a fresh override was applied.
pub async fn maybe_ai_override(
    pool: &PgPool,
    user_id: Uuid,
    weights: &mut HashMap<String, f64>,
    context_summary: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(chrono::DateTime<chrono::Utc>, i32, serde_json::Value)> = sqlx::query_as(
        "SELECT generated_at, uses_today, weight_deltas FROM wvi_ai_reweight_cache WHERE user_id=$1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    if let Some((gen_at, uses, _)) = &row {
        if Utc::now() - *gen_at < chrono::Duration::hours(4) {
            return Ok(None);
        }
        if *uses >= 3 {
            return Ok(None);
        }
    }

    let prompt = format!(
        "WVI v3 components & runtime context: {context_summary}.\n\
         Return JSON only, no markdown: \
         {{\"deltas\": {{<component>: <delta ±0.1>}}, \"rationale\": \"1 sentence RU\"}}"
    );
    let resp = crate::ai::cli::ask_or_fallback(crate::ai::cli::AiEndpointKind::Insights, &prompt).await;
    let parsed: serde_json::Value = match serde_json::from_str(&resp) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let deltas = parsed.get("deltas").cloned().unwrap_or(serde_json::json!({}));
    let rationale = parsed
        .get("rationale")
        .and_then(|v| v.as_str())
        .map(String::from);

    if let Some(obj) = deltas.as_object() {
        for (k, v) in obj {
            if let Some(d) = v.as_f64() {
                let clamped = d.clamp(-0.1, 0.1);
                if let Some(w) = weights.get_mut(k) {
                    *w = (*w + clamped).max(0.0);
                }
            }
        }
        renormalize(weights);
    }

    let uses = row.map(|(_, u, _)| u + 1).unwrap_or(1);
    sqlx::query(
        "INSERT INTO wvi_ai_reweight_cache (user_id, generated_at, weight_deltas, rationale, uses_today)
         VALUES ($1, NOW(), $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE SET
           generated_at=NOW(), weight_deltas=$2, rationale=$3, uses_today=$4",
    )
    .bind(user_id)
    .bind(&deltas)
    .bind(&rationale)
    .bind(uses)
    .execute(pool)
    .await?;

    Ok(rationale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renormalize_scales_to_1() {
        let mut w: HashMap<String, f64> =
            [("a".to_string(), 2.0), ("b".to_string(), 3.0)].into_iter().collect();
        renormalize(&mut w);
        let sum: f64 = w.values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn apply_context_sleep_boosts_sleep() {
        let mut w: HashMap<String, f64> = [
            ("sleep_composite".to_string(), 0.1),
            ("activity_personal".to_string(), 0.1),
            ("hrv_personal".to_string(), 0.1),
        ]
        .into_iter()
        .collect();
        let ctx = RuntimeContext {
            in_sleep_window: true,
            post_workout_minutes: -1,
            in_active_hours: false,
            illness_mode: false,
        };
        apply_context(&mut w, &ctx);
        assert!(w["sleep_composite"] > w["activity_personal"]);
    }
}
