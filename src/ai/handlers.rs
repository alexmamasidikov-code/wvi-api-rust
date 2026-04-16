use axum::{extract::State, Extension, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::cache::AppCache;
use crate::error::AppResult;
use super::cli::{ask_or_fallback, AiEndpointKind};
use super::context_builder::build_full_context;
use super::prompt_rules::WELLEX_SYSTEM_PROMPT;

/// Cache key: per (user, endpoint-kind). TTL is 10 minutes (AppCache::get_ai).
/// Combined with the 5-minute prewarm loop in `ai::precompute`, this means
/// panel endpoints are usually served in <50 ms instead of 20-40 s.
pub(crate) fn cache_key(privy_did: &str, kind: AiEndpointKind) -> String {
    format!("ai:{}:{}", privy_did, kind.as_str())
}

/// Unified entry: cache-read, miss → generate via call_claude, cache-write.
/// All panel handlers use this so the cache fills on first hit and the
/// prewarmer keeps it fresh.
pub(crate) async fn cached_call(
    cache: &AppCache,
    pool: &PgPool,
    privy_did: &str,
    kind: AiEndpointKind,
    prompt: &str,
) -> String {
    let key = cache_key(privy_did, kind);
    if let Some(hit) = cache.get_ai(&key).await {
        tracing::debug!(endpoint = ?kind, "AI cache hit for {}", privy_did);
        return hit;
    }
    let text = call_claude(pool, privy_did, kind, prompt).await;
    cache.set_ai(&key, text.clone()).await;
    text
}

// ─── Biometric context fetched from DB ───────────────────────────────────────

struct BiometricContext {
    heart_rate: Option<f32>,
    hrv_rmssd: Option<f32>,
    hrv_stress: Option<f32>,
    spo2: Option<f32>,
    temperature: Option<f32>,
    steps: Option<i32>,
    emotion: Option<String>,
    wvi_score: Option<f32>,
}

async fn fetch_biometrics(pool: &PgPool, privy_did: &str) -> BiometricContext {
    // Heart rate — latest bpm
    let heart_rate = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // HRV — latest rmssd and stress
    let hrv = sqlx::query_as::<_, (Option<f32>, Option<f32>)>(
        "SELECT rmssd, stress FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let (hrv_rmssd, hrv_stress) = hrv.unwrap_or((None, None));

    // SpO2 — latest value
    let spo2 = sqlx::query_scalar::<_, f32>(
        "SELECT value FROM spo2 WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // Temperature — latest value
    let temperature = sqlx::query_scalar::<_, f32>(
        "SELECT value FROM temperature WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // Activity — today's total steps
    let steps = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(SUM(steps), 0)::int FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '24 hours'"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // Emotion — latest dominant emotion
    let emotion = sqlx::query_scalar::<_, String>(
        "SELECT dominant_emotion FROM emotion_states WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // WVI score — latest
    let wvi_score = sqlx::query_scalar::<_, f32>(
        "SELECT score FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY calculated_at DESC LIMIT 1"
    )
    .bind(privy_did)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    BiometricContext {
        heart_rate,
        hrv_rmssd,
        hrv_stress,
        spo2,
        temperature,
        steps,
        emotion,
        wvi_score,
    }
}

fn format_biometric_context(ctx: &BiometricContext) -> String {
    let mut parts = vec!["Current biometric readings:".to_string()];

    match ctx.heart_rate {
        Some(v) => parts.push(format!("- Heart Rate: {:.0} bpm", v)),
        None => parts.push("- Heart Rate: no data".to_string()),
    }
    match ctx.hrv_rmssd {
        Some(v) => parts.push(format!("- HRV (RMSSD): {:.1} ms", v)),
        None => parts.push("- HRV: no data".to_string()),
    }
    match ctx.hrv_stress {
        Some(v) => parts.push(format!("- Stress Level: {:.0}/100", v)),
        None => {}
    }
    match ctx.spo2 {
        Some(v) => parts.push(format!("- SpO2: {:.1}%", v)),
        None => parts.push("- SpO2: no data".to_string()),
    }
    match ctx.temperature {
        Some(v) => parts.push(format!("- Body Temperature: {:.1}°C", v)),
        None => {}
    }
    match ctx.steps {
        Some(v) if v > 0 => parts.push(format!("- Steps today: {}", v)),
        _ => {}
    }
    match &ctx.emotion {
        Some(e) => parts.push(format!("- Current emotional state: {}", e)),
        None => {}
    }
    match ctx.wvi_score {
        Some(v) => parts.push(format!("- WVI Wellness Score: {:.1}/100", v)),
        None => {}
    }

    parts.join("\n")
}

// ─── Claude via CLI ──────────────────────────────────────────────────────────
//
// Calls the local `claude` CLI (Max subscription on production VPS) instead of
// HTTP to api.anthropic.com / OpenRouter. Always returns a user-facing string —
// falls back to per-endpoint static text if the CLI is unavailable.
// See `super::cli` module for the CLI wrapper.

pub(crate) async fn call_claude(pool: &PgPool, privy_did: &str, kind: AiEndpointKind, prompt: &str) -> String {
    // Rich multi-day DB context — 7-day averages + sleep history + WVI
    // trend + emotion distribution. Claude makes much better calls with
    // this than with a single snapshot.
    let db_context = build_full_context(pool, privy_did).await;

    // Quick derived metrics from latest snapshot (keep for backward-compat
    // with existing formatting; the full_context already includes values).
    let ctx = fetch_biometrics(pool, privy_did).await;
    let heart_rate = ctx.heart_rate.unwrap_or(70.0) as f64;
    let hrv = ctx.hrv_rmssd.unwrap_or(50.0) as f64;
    let spo2 = ctx.spo2.unwrap_or(98.0) as f64;
    let steps = ctx.steps.unwrap_or(0) as f64;

    let (sys, dia) = crate::biometrics::computed::estimate_blood_pressure(heart_rate, hrv);
    let vo2 = crate::biometrics::computed::estimate_vo2_max(heart_rate, 30.0);
    let coherence = crate::biometrics::computed::compute_coherence(hrv);
    let bio_age = crate::biometrics::computed::compute_bio_age(30.0, heart_rate, hrv, spo2, steps, 75.0);

    let mut computed_parts = vec!["### Computed estimates".to_string()];
    computed_parts.push(format!("- Estimated BP: **{:.0}/{:.0} mmHg**", sys, dia));
    computed_parts.push(format!("- Estimated VO2 Max: **{:.1} ml/kg/min**", vo2));
    computed_parts.push(format!("- Cardiac Coherence: **{:.0}%**", coherence));
    computed_parts.push(format!("- Estimated Bio Age: **{:.0} years**", bio_age));
    let computed_context = computed_parts.join("\n");

    // Final prompt: system rules + multi-day context + computed + task prompt.
    let full_prompt = format!(
        "{}\n\n---\n\n{}\n\n{}\n\n---\n\n## Task\n\n{}",
        WELLEX_SYSTEM_PROMPT, db_context, computed_context, prompt
    );

    ask_or_fallback(kind, &full_prompt).await
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn interpret(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Analyze ALL the biometric data above. For each metric, explain: 1) What the current value means 2) Whether it's in a healthy range 3) How it relates to other metrics. Start with the most important finding. Reference specific numbers.";
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::Interpret, prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn recommendations(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Based on ALL the biometric data, provide exactly 3 personalized recommendations. Each must: 1) Reference a specific metric value 2) Give a concrete action (not vague advice) 3) Explain the expected benefit. Format: numbered list with bold action.";
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::Recommendations, prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn chat(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let user_message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Tell me about my health.")
        .to_string();
    let prompt = format!(
        "The user asks: \"{}\"\n\nAnswer using the biometric context above to give a personalized, helpful response.",
        user_message
    );
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::Chat, &prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn explain_metric(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let metric = body
        .get("metric")
        .and_then(|v| v.as_str())
        .unwrap_or("heart rate")
        .to_string();
    let prompt = format!(
        "Explain the '{}' metric in the context of the biometric data above. What does the current value mean, what is optimal, and what affects it?",
        metric
    );
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::ExplainMetric, &prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn action_plan(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Create a personalized daily wellness plan based on the biometric data. Structure as:\n• MORNING (based on recovery/HRV): exercise type + duration\n• AFTERNOON (based on stress/activity): activity suggestion\n• EVENING (based on overall state): wind-down routine\nReference specific metric values to justify each suggestion.";
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::ActionPlan, prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn insights(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Identify the TOP 3 most significant findings from the biometric data. For each: 1) What you found (with specific numbers) 2) Why it matters 3) What to do about it. Prioritize by health impact. Use bullet points.";
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::Insights, prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn genius_layer(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "You are the Genius Layer — provide the deepest analysis possible. Connect ALL signals:\n1) How HR + HRV + Stress interact (autonomic nervous system state)\n2) How SpO2 + Temperature + Activity relate (metabolic state)\n3) How Emotional state connects to physiological data\n4) What the WVI score + Bio Age reveal about long-term trajectory\n5) One non-obvious insight that connects 3+ metrics\nBe specific with numbers. This is the premium analysis.";
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::GeniusLayer, prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

// ─── AI Coach 2.0 (proactive) ────────────────────────────────────────────────

/// Prompt for the daily morning brief — extracted so the background
/// prewarmer (`ai::precompute`) calls the exact same text.
pub(crate) const DAILY_BRIEF_PROMPT: &str = "Write a 3-sentence morning brief for this user. Start with a 🌅 emoji. \
Open with one observation about their overnight recovery (HRV vs baseline / sleep \
quality). Follow with the single most valuable action they should take today. \
Close with a concrete intensity ceiling for training (or 'rest day' if HRV is \
significantly below baseline). Tone: warm coach, not clinical.";

pub async fn daily_brief(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::DailyBrief, DAILY_BRIEF_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub(crate) const EVENING_REVIEW_PROMPT: &str = "Write a short evening review: \
1) **What went right today** (one sentence, cite a metric). \
2) **What could improve** (one sentence, cite a metric). \
3) **Tonight's wind-down** (one specific action: bedtime target, breathing \
pattern, or caffeine/light discipline). \
Keep it under 100 words. Warm, supportive.";

pub async fn evening_review(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::EveningReview, EVENING_REVIEW_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn anomaly_alert(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let metric = body.get("metric").and_then(|v| v.as_str()).unwrap_or("a biometric signal");
    let direction = body.get("direction").and_then(|v| v.as_str()).unwrap_or("shifted");
    let delta = body.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let prompt = format!(
        "A significant change was detected: **{}** {} by {:.1}% vs the user's 7-day baseline. \
        Write a 2-sentence push-notification body (max 140 chars): \
        explain what this likely means physiologically + one immediate action. \
        No emoji. No alarmism. If the change is within normal daily variation, say so briefly.",
        metric, direction, delta.abs()
    );
    let text = call_claude(&pool, &user.privy_did, AiEndpointKind::AnomalyAlert, &prompt).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub(crate) const WEEKLY_DEEP_PROMPT: &str = "Produce the Sunday weekly deep analysis, 5 sections: \
1) **WVI summary**: trend + biggest contributor + biggest drag (cite numbers). \
2) **Sleep architecture**: average duration, deep/REM percentages vs targets. \
3) **Autonomic balance**: HRV trend vs stress; recovery pattern. \
4) **Activity + recovery load**: ACWR, training response, overreach risk. \
5) **Next week's focus**: 1-3 SMART goals derived from this week's gaps. \
Use Markdown headers. This is the premium weekly report.";

pub async fn weekly_deep(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::WeeklyDeep, WEEKLY_DEEP_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

// ─── Medical-analyst tier ───────────────────────────────────────────────────

pub(crate) const FULL_ANALYSIS_PROMPT: &str = "You are performing a **complete medical-analyst review** of this user's \
recent biometric data. Use the full dossier provided (latest snapshot, 7-day averages, \
sleep architecture, WVI trend + per-metric breakdown, emotion distribution, ECG sessions, \
activity today). \
\n\n\
Produce the report in this structure: \
\n## Cardiovascular\n(HR, HRV, BP estimate, resting HR trend, any irregular patterns) \
\n## Respiration & metabolic\n(SpO2, breathing rate, temperature, VO2 Max) \
\n## Autonomic / stress\n(HRV vs baseline, stress index pattern, PPI coherence) \
\n## Sleep & recovery\n(duration, phase distribution, efficiency, debt) \
\n## Activity & training\n(steps, active minutes, ACWR, overreach risk) \
\n## Emotional signature\n(primary + secondary emotions, correlations with physiology) \
\n## ECG findings\n(if recent ECG: rate, rhythm hints, any flags) \
\n## Three highest-leverage actions\n(specific, numeric, ordered by impact) \
\nCite real numbers everywhere. Flag anything outside reference range. Add a brief \
professional-help note only if genuinely warranted (per system rules).";

pub async fn full_analysis(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::FullAnalysis, FULL_ANALYSIS_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub(crate) const ECG_INTERPRET_PROMPT: &str = "Interpret the user's most recent ECG session. Cover: \
1) **Rate**: what the HR during the recording tells you (resting vs activity). \
2) **Rhythm quality**: regular vs irregular signatures based on the analysis JSON \
blob in the dossier. \
3) **PPI coherence**: cardiac coherence reading and what it suggests about autonomic \
tone. \
4) **Integration**: how this ECG fits with today's HRV, stress, and emotional state. \
5) **Action**: what to track next (re-measure after a specific activity / at a \
specific time of day). \
\n\n\
If the ECG analysis blob is missing or the recording too short for confident analysis, \
say so explicitly and suggest a fresh 60-second measurement with the Wellex bracelet.";

pub async fn ecg_interpret(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::EcgInterpret, ECG_INTERPRET_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub(crate) const RECOVERY_DEEP_PROMPT: &str = "Produce a deep recovery analysis: \
1) **Current recovery state**: % based on morning HRV vs 7-day baseline + previous \
night's sleep score + stress trend. \
2) **Autonomic balance**: parasympathetic vs sympathetic read based on HRV trajectory. \
3) **Sleep contribution**: deep + REM percentages vs targets; note if debt is building. \
4) **Training readiness**: green/yellow/red with specific intensity ceiling. \
5) **Recovery prescription**: 3 concrete actions for the next 24h ordered by impact. \
\nBe specific with numbers. If recovery data is sparse, say what's missing \
(e.g. 'need one more night of sleep tracking').";

pub async fn recovery_deep(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_call(&cache, &pool, &user.privy_did, AiEndpointKind::RecoveryDeep, RECOVERY_DEEP_PROMPT).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}
