use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use super::cli::{ask_or_fallback, AiEndpointKind};

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

async fn call_claude(pool: &PgPool, privy_did: &str, kind: AiEndpointKind, prompt: &str) -> String {
    let ctx = fetch_biometrics(pool, privy_did).await;
    let bio_context = format_biometric_context(&ctx);

    // Compute derived metrics for richer AI context
    let heart_rate = ctx.heart_rate.unwrap_or(70.0) as f64;
    let hrv = ctx.hrv_rmssd.unwrap_or(50.0) as f64;
    let spo2 = ctx.spo2.unwrap_or(98.0) as f64;
    let steps = ctx.steps.unwrap_or(0) as f64;

    let (sys, dia) = crate::biometrics::computed::estimate_blood_pressure(heart_rate, hrv);
    let vo2 = crate::biometrics::computed::estimate_vo2_max(heart_rate, 30.0);
    let coherence = crate::biometrics::computed::compute_coherence(hrv);
    let bio_age = crate::biometrics::computed::compute_bio_age(30.0, heart_rate, hrv, spo2, steps, 75.0);

    let mut computed_parts = vec!["Computed estimates:".to_string()];
    computed_parts.push(format!("- Estimated BP: {:.0}/{:.0} mmHg", sys, dia));
    computed_parts.push(format!("- Estimated VO2 Max: {:.1} ml/kg/min", vo2));
    computed_parts.push(format!("- Cardiac Coherence: {:.0}%", coherence));
    computed_parts.push(format!("- Estimated Bio Age: {:.0} years", bio_age));
    let computed_context = computed_parts.join("\n");

    let system_prompt = format!(
        r#"You are Wellex AI — a personal wellness intelligence assistant inside the Wellex health app.

## Your Role
- You analyze real-time biometric data from the user's Wellex bracelet (JCV8)
- You provide personalized, evidence-based health insights
- You speak confidently but add disclaimers for medical concerns
- You are warm, supportive, and actionable — never alarmist
- You reference specific numbers from the user's data
- Respond in 3-5 sentences. Use bullet points for lists.

## WVI Score System
- WVI (Wellness Vitality Index) is 0-100, calculated from 10 components:
  HRV (18%), Stress (15%), Sleep (13%), Emotion (12%), SpO2 (9%), HR (9%), Activity (8%), BP (6%), Temp (5%), PPI (5%)
- Levels: Superb (90+), Excellent (80-89), Good (65-79), Moderate (50-64), Attention (35-49), Critical (20-34), Dangerous (<20)

## 18 Emotional States
Wellex detects emotions via fuzzy logic: Calm, Relaxed, Joyful, Energized, Excited, Focused, Flow, Meditative, Recovering, Drowsy, Sad, Anxious, Stressed, Frustrated, Angry, Fearful, Exhausted, Pain

## Key Metric Ranges
- HR: 60-80 bpm (rest optimal), >100 elevated
- HRV: >50ms good, >70ms excellent, <20ms poor
- SpO2: >95% normal, <90% critical
- Stress: 0-25 low, 25-50 moderate, 50-75 high, 75-100 very high
- Recovery: based on morning HRV vs 7-day baseline
- Bio Age: calculated from all metrics over 7 days

## Current User Data
{}

{}"#,
        bio_context, computed_context
    );

    // Assemble the full prompt: system + user question. Claude CLI reads a
    // single combined prompt from stdin/argv — no separate system/user roles.
    let full_prompt = format!("{}\n\n---\n\n{}", system_prompt, prompt);

    // ask_or_fallback never panics; on CLI error it returns the per-endpoint
    // static fallback text so iOS always gets something renderable.
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
