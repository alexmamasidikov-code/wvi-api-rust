use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Extension, Json,
};
use futures::stream::StreamExt;
use sqlx::PgPool;
use std::convert::Infallible;
use crate::auth::middleware::AuthUser;
use crate::cache::AppCache;
use crate::error::AppResult;
use super::cli::{
    ask_or_fallback, invoke_ai_chat_with_context, invoke_ai_kind, invoke_claude_cli_streaming,
    AiEndpointKind,
};
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
#[tracing::instrument(
    name = "ai.cached_call",
    skip_all,
    fields(kind = kind.as_str(), user = privy_did, cache_hit = tracing::field::Empty)
)]
pub(crate) async fn cached_call(
    cache: &AppCache,
    pool: &PgPool,
    privy_did: &str,
    kind: AiEndpointKind,
    prompt: &str,
) -> String {
    let span = tracing::Span::current();
    let key = cache_key(privy_did, kind);
    if let Some(hit) = cache.get_ai(&key).await {
        span.record("cache_hit", true);
        tracing::debug!(endpoint = ?kind, "AI cache hit for {}", privy_did);
        if let Some(m) = crate::metrics::global() {
            m.ai_cache_hits.inc();
        }
        return hit;
    }
    span.record("cache_hit", false);
    if let Some(m) = crate::metrics::global() {
        m.ai_cache_misses.inc();
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

/// Build the full biometric + computed context string used as the
/// `biometrics` Mustache variable for /v1/kind/wvi.* templates.
pub(crate) async fn build_biometrics_block(pool: &PgPool, privy_did: &str) -> String {
    let db_context = build_full_context(pool, privy_did).await;

    let ctx = fetch_biometrics(pool, privy_did).await;
    let heart_rate = ctx.heart_rate.unwrap_or(70.0) as f64;
    let hrv = ctx.hrv_rmssd.unwrap_or(50.0) as f64;
    let spo2 = ctx.spo2.unwrap_or(98.0) as f64;
    let steps = ctx.steps.unwrap_or(0) as f64;

    let (sys, dia) = crate::biometrics::computed::estimate_blood_pressure(heart_rate, hrv);
    let vo2 = crate::biometrics::computed::estimate_vo2_max(heart_rate, 30.0);
    let coherence = crate::biometrics::computed::compute_coherence(hrv);
    let bio_age = crate::biometrics::computed::compute_bio_age(30.0, heart_rate, hrv, spo2, steps, 75.0);

    let mut computed = vec!["### Computed estimates".to_string()];
    computed.push(format!("- Estimated BP: **{:.0}/{:.0} mmHg**", sys, dia));
    computed.push(format!("- Estimated VO2 Max: **{:.1} ml/kg/min**", vo2));
    computed.push(format!("- Cardiac Coherence: **{:.0}%**", coherence));
    computed.push(format!("- Estimated Bio Age: **{:.0} years**", bio_age));

    format!("{}\n\n{}", db_context, computed.join("\n"))
}

/// Canonical AI handler path: build biometric context, call the named prompt
/// template on the gateway (`/v1/kind/<template>`), fall back to the legacy
/// `call_claude` CLI path on gateway error, fall back to static text last.
pub(crate) async fn call_kind(
    pool: &PgPool,
    privy_did: &str,
    kind: AiEndpointKind,
    template: &str,
    extra_vars: serde_json::Value,
) -> String {
    let biometrics = build_biometrics_block(pool, privy_did).await;
    let mut vars = match extra_vars {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    vars.insert("biometrics".to_string(), serde_json::Value::String(biometrics));

    match invoke_ai_kind(template, serde_json::Value::Object(vars)).await {
        Ok(text) => text,
        Err(reason) => {
            tracing::warn!(endpoint = ?kind, template, reason = %reason, "kind call failed, using fallback");
            kind.fallback_text().to_string()
        }
    }
}

/// Same as `call_kind` but wrapped in AppCache (10-minute TTL). Used by
/// cache-friendly panels (daily_brief, body_story, weekly_deep, recovery_deep,
/// full_analysis, ecg_interpret narrative, evening_review).
pub(crate) async fn cached_kind_call(
    cache: &AppCache,
    pool: &PgPool,
    privy_did: &str,
    kind: AiEndpointKind,
    template: &str,
    extra_vars: serde_json::Value,
) -> String {
    let key = cache_key(privy_did, kind);
    if let Some(hit) = cache.get_ai(&key).await {
        if let Some(m) = crate::metrics::global() {
            m.ai_cache_hits.inc();
        }
        return hit;
    }
    if let Some(m) = crate::metrics::global() {
        m.ai_cache_misses.inc();
    }
    let text = call_kind(pool, privy_did, kind, template, extra_vars).await;
    cache.set_ai(&key, text.clone()).await;
    text
}

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
    let text = call_kind(&pool, &user.privy_did, AiEndpointKind::Interpret, "wvi.interpret", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn recommendations(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = call_kind(&pool, &user.privy_did, AiEndpointKind::Recommendations, "wvi.recommendations", serde_json::json!({})).await;
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
    let text = call_kind(
        &pool,
        &user.privy_did,
        AiEndpointKind::Chat,
        "wvi.chat",
        serde_json::json!({ "message": user_message }),
    )
    .await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

/// Streaming variant of `chat`. Yields SSE `data:` frames with the text
/// delta as the Claude CLI emits it, so iOS can render token-by-token
/// instead of blocking for 20-30 s. Falls back to a single synthesised
/// frame with the static fallback text if streaming produces nothing
/// within the CLI timeout (e.g. CLI missing on the VPS).
///
/// Frame shape: `data: {"delta": "..."}\n\n` per delta,
/// terminated by `data: [DONE]\n\n`.
pub async fn chat_stream(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let user_message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Tell me about my health.")
        .to_string();

    // Assemble the same biometric-aware prompt as the non-streaming path
    // so answers stay consistent. The system prompt is prepended inline
    // because the CLI streaming invoke doesn't re-read WELLEX_SYSTEM_PROMPT
    // through cached_call.
    let ctx = build_full_context(&pool, &user.privy_did).await;
    let prompt = format!(
        "{}\n\n{}\n\nThe user asks: \"{}\"\n\nAnswer using the biometric context above to give a personalized, helpful response.",
        WELLEX_SYSTEM_PROMPT, ctx, user_message
    );

    let privy_did = user.privy_did.clone();
    let stream = async_stream::stream! {
        let mut delivered_any = false;
        let mut inner = Box::pin(invoke_claude_cli_streaming(&prompt));
        while let Some(chunk) = inner.next().await {
            delivered_any = true;
            let payload = serde_json::json!({ "delta": chunk }).to_string();
            yield Ok::<_, Infallible>(Event::default().data(payload));
        }
        // If the CLI yielded nothing (missing, timeout, error), hand
        // the caller the static fallback in one frame so the UI isn't
        // left blank.
        if !delivered_any {
            let fallback = AiEndpointKind::Chat.fallback_text();
            tracing::warn!(user = %privy_did, "chat_stream produced no chunks; sending fallback");
            let payload = serde_json::json!({ "delta": fallback }).to_string();
            yield Ok::<_, Infallible>(Event::default().data(payload));
        }
        yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
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
    let text = call_kind(
        &pool,
        &user.privy_did,
        AiEndpointKind::ExplainMetric,
        "wvi.explain_metric",
        serde_json::json!({ "metric": metric }),
    )
    .await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn action_plan(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = call_kind(&pool, &user.privy_did, AiEndpointKind::ActionPlan, "wvi.action_plan", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn insights(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = call_kind(&pool, &user.privy_did, AiEndpointKind::Insights, "wvi.insights", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn genius_layer(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let text = call_kind(&pool, &user.privy_did, AiEndpointKind::GeniusLayer, "wvi.genius_layer", serde_json::json!({})).await;
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
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::DailyBrief, "wvi.daily_brief", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub(crate) const BODY_STORY_PROMPT: &str = "Write a 2-sentence \"body story\" \
for the BODY tab of this app — a calm, plain-language read of how the user's body \
is doing right now. Sentence 1: cite ONE specific cardiac/sleep/stress signal with \
its current value and what it implies (\"HR 65 bpm — your body is in a parasympathetic \
state\"). Sentence 2: ONE concrete suggestion grounded in that observation \
(walk, breathing, hydration, rest). Tone: calm coach. No emoji. No clinical jargon \
the user wouldn't recognise. Max 35 words.";

pub async fn body_story(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::BodyStory, "wvi.body_story", serde_json::json!({})).await;
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
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::EveningReview, "wvi.evening_review", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}

pub async fn anomaly_alert(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let metric = body.get("metric").and_then(|v| v.as_str()).unwrap_or("a biometric signal").to_string();
    let direction = body.get("direction").and_then(|v| v.as_str()).unwrap_or("shifted").to_string();
    let delta = body.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0).abs();
    let text = call_kind(
        &pool,
        &user.privy_did,
        AiEndpointKind::AnomalyAlert,
        "wvi.anomaly_alert",
        serde_json::json!({
            "metric": metric,
            "direction": direction,
            "delta_pct": format!("{:.1}", delta),
        }),
    )
    .await;
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
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::WeeklyDeep, "wvi.weekly_deep", serde_json::json!({})).await;
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
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::FullAnalysis, "wvi.full_analysis", serde_json::json!({})).await;
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

/// Body schema for the Project-F strict-JSON ECG interpretation path.
/// All fields optional so the legacy "ask about the latest ECG" call
/// (no body) still works and falls through to the cached narrative prompt.
#[derive(Debug, serde::Deserialize, Default)]
#[serde(default)]
pub struct ECGInterpretBody {
    pub samples: Option<Vec<f64>>,
    #[serde(alias = "durationSeconds")]
    pub duration_seconds: Option<i32>,
    #[serde(alias = "sampleRate")]
    pub sample_rate: Option<i32>,
    #[serde(alias = "ecgId")]
    pub ecg_id: Option<uuid::Uuid>,
}

pub async fn ecg_interpret(
    user: AuthUser,
    Extension(cache): Extension<AppCache>,
    State(pool): State<PgPool>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    // Two modes:
    //   1. Legacy (no samples): run the cached prose prompt against the
    //      user's overall biometric context. This is what iOS panels call
    //      when the user taps "Interpret my latest ECG" from the AI dossier.
    //   2. Project-F strict-JSON (samples supplied): run a one-shot prompt
    //      that must return a JSON blob matching the schema, persist it to
    //      ecg.analysis_json, and fire a crisis push if is_crisis is set.
    let body: ECGInterpretBody = serde_json::from_value(body).unwrap_or_default();

    let Some(samples) = body.samples else {
        // Legacy fall-through path — switched to /v1/kind/wvi.ecg_interpret.
        let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::EcgInterpret, "wvi.ecg_interpret", serde_json::json!({})).await;
        return Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })));
    };

    let duration_seconds = body.duration_seconds.unwrap_or(30);
    let sample_rate = body.sample_rate.unwrap_or(125);
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await
        .map_err(|e| crate::error::AppError::Database(e))?;

    let prompt = crate::ai::cli::ecg_interpret_prompt(&samples, duration_seconds, sample_rate);

    // First attempt — raw CLI call.
    let first = crate::ai::cli::invoke_claude_cli(&prompt).await;
    let parsed: Option<serde_json::Value> = first.as_ref().ok()
        .and_then(|raw| extract_first_json_object(raw));

    let analysis = if let Some(v) = parsed {
        v
    } else {
        // Retry once with an explicit JSON-only reminder appended.
        match crate::ai::cli::invoke_claude_cli_retry(
            &prompt,
            "Your previous response was not parseable JSON. Respond ONLY with the JSON object. No markdown, no commentary."
        ).await {
            Ok(raw) => extract_first_json_object(&raw).unwrap_or_else(|| {
                tracing::warn!("ecg_interpret: retry also failed to yield JSON, falling back to null analysis");
                serde_json::Value::Null
            }),
            Err(reason) => {
                tracing::warn!(?reason, "ecg_interpret: CLI retry errored, falling back to null analysis");
                serde_json::Value::Null
            }
        }
    };

    // Persist into ecg.analysis_json so GET /biometrics/ecg returns enriched data.
    if let (Some(ecg_id), true) = (body.ecg_id, !analysis.is_null()) {
        if let Err(e) = sqlx::query(
            "UPDATE ecg SET analysis_json = $1 WHERE id = $2 AND user_id = $3"
        )
        .bind(&analysis)
        .bind(ecg_id)
        .bind(user_id)
        .execute(&pool).await {
            tracing::warn!(?e, ?ecg_id, "ecg_interpret: failed to persist analysis_json");
        }
    }

    // Crisis dispatch — fire-and-forget so the HTTP response is fast.
    if analysis.get("is_crisis").and_then(|v| v.as_bool()).unwrap_or(false) {
        let ecg_id = body.ecg_id;
        let pool_c = pool.clone();
        let analysis_c = analysis.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatch_ecg_crisis(&pool_c, user_id, ecg_id, &analysis_c).await {
                tracing::warn!(?e, "dispatch_ecg_crisis failed");
            }
        });
    }

    Ok(Json(serde_json::json!({ "success": true, "data": analysis })))
}

/// Claude occasionally wraps the JSON in ```json fences or adds a leading
/// apology line. Extract the first balanced `{...}` block and try to parse
/// that instead of failing the whole pipeline.
fn extract_first_json_object(raw: &str) -> Option<serde_json::Value> {
    // Fast path: the output is already parseable.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw.trim()) {
        return Some(v);
    }
    // Walk the string and track brace depth until we close one balanced object.
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape { escape = false; }
            else if b == b'\\' { escape = true; }
            else if b == b'"' { in_string = false; }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let slice = &raw[start..=i];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) {
                        return Some(v);
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// Send an ECG crisis APNs push with a 4h dedup window. Reuses the
/// `push_notifications_log` table and the `ApnsClient::send_alert` pattern
/// that Project D established for BP crisis pushes.
async fn dispatch_ecg_crisis(
    pool: &PgPool,
    user_id: uuid::Uuid,
    ecg_id: Option<uuid::Uuid>,
    analysis: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let last: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
        "SELECT sent_at FROM push_notifications_log
         WHERE user_id=$1 AND category='ecg_crisis'
         ORDER BY sent_at DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await?;
    if let Some((last_ts,)) = last {
        if chrono::Utc::now() - last_ts < chrono::Duration::hours(4) {
            tracing::info!(?user_id, "ecg_crisis dedup: last push <4h ago");
            return Ok(());
        }
    }

    let hr = analysis
        .pointer("/metrics/hr_mean")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let title = if hr > 0 {
        format!("⚠️ ECG требует внимания — HR {} bpm", hr)
    } else {
        "⚠️ ECG требует внимания врача".to_string()
    };
    let body_text = analysis
        .get("recommendation")
        .and_then(|v| v.as_str())
        .unwrap_or("Обратись к врачу — запись ECG вне нормы")
        .to_string();

    let tokens: Vec<String> = sqlx::query_scalar(
        "SELECT token FROM push_tokens WHERE user_id=$1"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if tokens.is_empty() {
        tracing::info!(?user_id, "ecg_crisis: no push tokens — skip");
        return Ok(());
    }

    let apns = crate::push::apns::ApnsClient::new();
    let deeplink = match ecg_id {
        Some(id) => format!("wellex://body/ecg/{}", id),
        None => "wellex://body/ecg".to_string(),
    };
    let mut sent = 0;
    for token in &tokens {
        match apns.send_alert(token, &title, &body_text, Some(&deeplink)).await {
            Ok(()) => sent += 1,
            Err(e) => tracing::warn!(?user_id, "apns ecg_crisis send failed: {e}"),
        }
    }

    if sent > 0 {
        sqlx::query(
            "INSERT INTO push_notifications_log (user_id, category) VALUES ($1, 'ecg_crisis')"
        ).bind(user_id).execute(pool).await?;
    }

    Ok(())
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
    let text = cached_kind_call(&cache, &pool, &user.privy_did, AiEndpointKind::RecoveryDeep, "wvi.recovery_deep", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
}
