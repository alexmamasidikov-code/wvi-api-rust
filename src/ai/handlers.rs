use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

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

// ─── Claude API call ──────────────────────────────────────────────────────────

async fn call_claude(pool: &PgPool, privy_did: &str, prompt: &str) -> Result<String, String> {
    let api_key = std::env::var("CLAUDE_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        return Ok(
            "AI features require an API key. Set CLAUDE_API_KEY in your .env file.".to_string(),
        );
    }

    let model = std::env::var("CLAUDE_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.0-flash-001".to_string());

    let api_url = std::env::var("CLAUDE_API_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());

    let ctx = fetch_biometrics(pool, privy_did).await;
    let bio_context = format_biometric_context(&ctx);

    let system_prompt = format!(
        "You are Wellex AI, a personal wellness assistant. You analyze biometric data and provide health insights. Be concise, supportive, and actionable. Respond in 2-3 sentences max.\n\n{}",
        bio_context
    );

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 300,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": prompt}
        ]
    });

    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to reach AI API: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("AI API error {}: {}", status, text));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse AI response: {}", e))?;

    // OpenRouter / OpenAI format: choices[0].message.content
    let text = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or("No response from AI.")
        .to_string();

    Ok(text)
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub async fn interpret(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Interpret these biometrics and explain what they mean for the user's health. Be clear, concise, and supportive.";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}

pub async fn recommendations(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Based on these biometrics, provide 3 specific, actionable health recommendations. Number each one.";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
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
    match call_claude(&pool, &user.privy_did, &prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
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
    match call_claude(&pool, &user.privy_did, &prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}

pub async fn action_plan(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Create a practical daily action plan based on these biometrics. Include morning, afternoon, and evening suggestions. Keep it realistic and specific.";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}

pub async fn insights(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Identify key patterns and insights from these biometrics. Highlight anything notable, concerning, or positive. Focus on what stands out.";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}

pub async fn genius_layer(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let prompt = "Provide an advanced holistic analysis connecting all biometric signals. Identify interdependencies between heart rate, HRV, SpO2, temperature, activity, and emotional state. What does the full picture reveal about this person's current health and wellness trajectory?";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } }))),
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}
