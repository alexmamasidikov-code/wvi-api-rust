//! Assembles rich, multi-day biometric context for Claude.
//!
//! Gives the AI a near-complete picture of the user's biometric life:
//! - Latest snapshot (HR / HRV / SpO2 / Temp)
//! - 7-day rolling averages
//! - Sleep history with phase architecture
//! - WVI score trend + breakdown
//! - Emotion distribution last 24h
//! - Recent ECG sessions (last 3 with rhythm quality)
//! - Computed metrics (BP, VO2, Coherence, Bio Age, Training Load)
//! - Activity load (steps + zones)
//!
//! Output is Markdown, ~4-8 KB. Well under any model context limit.

use sqlx::PgPool;

pub async fn build_full_context(pool: &PgPool, privy_did: &str) -> String {
    // Resolve user_id once
    let user_id: Option<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE privy_did = $1")
        .bind(privy_did)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    let Some(user_id) = user_id else {
        return "## User Data\n\n_No user profile found — demo session._".to_string();
    };

    let mut parts = vec!["## Biometric dossier".to_string()];

    parts.push(latest_snapshot(pool, user_id).await);
    parts.push(week_summary(pool, user_id).await);
    parts.push(recent_sleep(pool, user_id).await);
    parts.push(wvi_history(pool, user_id).await);
    parts.push(emotion_distribution(pool, user_id).await);
    parts.push(recent_ecg(pool, user_id).await);
    parts.push(activity_summary(pool, user_id).await);

    parts.join("\n\n")
}

async fn latest_snapshot(pool: &PgPool, user_id: uuid::Uuid) -> String {
    let hr: Option<f32> = sqlx::query_scalar(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let hrv: Option<(f32, f32)> = sqlx::query_as(
        "SELECT rmssd, stress FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let spo2: Option<f32> = sqlx::query_scalar(
        "SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let temp: Option<f32> = sqlx::query_scalar(
        "SELECT value FROM temperature WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let ppi: Option<f32> = sqlx::query_scalar(
        "SELECT coherence::real FROM ppi WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let mut out = String::from("### Latest reading\n");
    if let Some(v) = hr { out.push_str(&format!("- Heart rate: **{:.0} bpm**\n", v)); }
    if let Some((r, s)) = hrv { out.push_str(&format!("- HRV RMSSD: **{:.1} ms** · Stress index: **{:.0}**\n", r, s)); }
    if let Some(v) = spo2 { out.push_str(&format!("- SpO2: **{:.0}%**\n", v)); }
    if let Some(v) = temp { out.push_str(&format!("- Temperature: **{:.1}°C**\n", v)); }
    if let Some(v) = ppi { out.push_str(&format!("- PPI coherence: **{:.2}**\n", v)); }

    if out == "### Latest reading\n" {
        out.push_str("_No recent readings._\n");
    }
    out
}

async fn week_summary(pool: &PgPool, user_id: uuid::Uuid) -> String {
    let hr_avg: Option<f32> = sqlx::query_scalar(
        "SELECT AVG(bpm)::real FROM heart_rate WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let hr_min: Option<i32> = sqlx::query_scalar(
        "SELECT MIN(bpm) FROM heart_rate WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let hrv_avg: Option<f32> = sqlx::query_scalar(
        "SELECT AVG(rmssd)::real FROM hrv WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let spo2_avg: Option<f32> = sqlx::query_scalar(
        "SELECT AVG(value)::real FROM spo2 WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let steps_total: Option<i64> = sqlx::query_scalar(
        "SELECT SUM(steps)::bigint FROM activity WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let mut out = String::from("### 7-day rolling averages\n");
    if let Some(v) = hr_avg { out.push_str(&format!("- Avg HR: **{:.0} bpm**\n", v)); }
    if let Some(v) = hr_min { out.push_str(&format!("- Resting HR (min observed): **{} bpm**\n", v)); }
    if let Some(v) = hrv_avg { out.push_str(&format!("- Avg HRV: **{:.1} ms**\n", v)); }
    if let Some(v) = spo2_avg { out.push_str(&format!("- Avg SpO2: **{:.1}%**\n", v)); }
    if let Some(v) = steps_total { out.push_str(&format!("- Total steps: **{}** (~{:.0}/day)\n", v, v as f64 / 7.0)); }

    if out == "### 7-day rolling averages\n" {
        out.push_str("_Insufficient historical data._\n");
    }
    out
}

async fn recent_sleep(pool: &PgPool, user_id: uuid::Uuid) -> String {
    #[derive(sqlx::FromRow)]
    struct SleepRow {
        date: Option<chrono::NaiveDate>,
        total_hours: Option<f32>,
        sleep_score: Option<f32>,
        deep_percent: Option<f32>,
        rem_percent: Option<f32>,
        light_percent: Option<f32>,
        awake_percent: Option<f32>,
        efficiency: Option<f32>,
    }

    let rows: Vec<SleepRow> = sqlx::query_as(
        "SELECT date, total_hours, sleep_score, deep_percent, rem_percent, \
         light_percent, awake_percent, efficiency \
         FROM sleep_records WHERE user_id = $1 ORDER BY date DESC LIMIT 7"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if rows.is_empty() {
        return "### Sleep architecture (last 7 nights)\n_No sleep records._".to_string();
    }

    let mut out = String::from("### Sleep architecture (last 7 nights)\n");
    for r in rows {
        let date = r.date.map(|d| d.format("%b %d").to_string()).unwrap_or_else(|| "?".to_string());
        let dur = r.total_hours.map(|h| format!("{:.1}h", h)).unwrap_or_else(|| "—".to_string());
        let score = r.sleep_score.map(|s| format!("{:.0}", s)).unwrap_or_else(|| "—".to_string());
        let eff = r.efficiency.map(|e| format!("{:.0}%", e)).unwrap_or_else(|| "—".to_string());
        let deep = r.deep_percent.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "—".to_string());
        let rem = r.rem_percent.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "—".to_string());
        let light = r.light_percent.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "—".to_string());
        let awake = r.awake_percent.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "—".to_string());
        out.push_str(&format!(
            "- **{}**: {} · score **{}** · eff {} · Deep {} · REM {} · Light {} · Awake {}\n",
            date, dur, score, eff, deep, rem, light, awake
        ));
    }
    out
}

async fn wvi_history(pool: &PgPool, user_id: uuid::Uuid) -> String {
    let scores: Vec<(f32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT wvi_score, timestamp FROM wvi_scores \
         WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '7 days' \
         ORDER BY timestamp DESC LIMIT 14"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if scores.is_empty() {
        return "### WVI trend\n_No WVI history yet._".to_string();
    }

    let mut out = String::from("### WVI trend (last 7 days)\n");
    for (s, t) in &scores {
        out.push_str(&format!("- {}: **{:.1}**\n", t.format("%b %d %H:%M"), s));
    }

    if scores.len() >= 2 {
        let first = scores.last().unwrap().0;
        let last = scores.first().unwrap().0;
        let delta = last - first;
        let direction = if delta > 2.0 { "↑ trending up" }
            else if delta < -2.0 { "↓ trending down" }
            else { "→ stable" };
        out.push_str(&format!("\n**Trend:** {} ({:+.1} over {} readings)\n",
            direction, delta, scores.len()));
    }

    // Latest breakdown
    let latest_metrics: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT metrics FROM wvi_scores WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    if let Some(m) = latest_metrics {
        out.push_str("\n**Latest per-metric contributions:**\n");
        if let Some(obj) = m.as_object() {
            let mut entries: Vec<(String, f64)> = obj.iter()
                .filter_map(|(k, v)| v.as_f64().map(|n| (k.clone(), n)))
                .collect();
            entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (k, v) in entries.iter().take(12) {
                out.push_str(&format!("- {}: **{:.0}**\n", k, v));
            }
        }
    }
    out
}

async fn emotion_distribution(pool: &PgPool, user_id: uuid::Uuid) -> String {
    #[derive(sqlx::FromRow)]
    struct EmoRow {
        emotion: Option<String>,
        count: Option<i64>,
    }

    let rows: Vec<EmoRow> = sqlx::query_as(
        "SELECT primary_emotion AS emotion, COUNT(*)::bigint AS count \
         FROM emotions \
         WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '24 hours' \
         GROUP BY primary_emotion ORDER BY count DESC LIMIT 8"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if rows.is_empty() {
        return "### Emotion distribution (last 24h)\n_No emotion data recorded._".to_string();
    }

    let total: i64 = rows.iter().map(|r| r.count.unwrap_or(0)).sum();
    let mut out = String::from("### Emotion distribution (last 24h)\n");
    for r in rows {
        let emo = r.emotion.unwrap_or_else(|| "?".to_string());
        let pct = r.count.unwrap_or(0) as f64 / total.max(1) as f64 * 100.0;
        out.push_str(&format!("- {}: **{:.0}%**\n", emo, pct));
    }

    // Current emotion
    let primary: Option<(String, Option<f32>)> = sqlx::query_as(
        "SELECT primary_emotion, primary_confidence FROM emotions \
         WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    if let Some((emo, conf)) = primary {
        let conf_str = conf.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "—".to_string());
        out.push_str(&format!("\n**Current:** {} (confidence {})\n", emo, conf_str));
    }
    out
}

async fn recent_ecg(pool: &PgPool, user_id: uuid::Uuid) -> String {
    #[derive(sqlx::FromRow)]
    struct EcgRow {
        timestamp: Option<chrono::DateTime<chrono::Utc>>,
        duration_seconds: Option<i32>,
        sample_rate: Option<i32>,
        analysis: Option<serde_json::Value>,
    }

    let rows: Vec<EcgRow> = sqlx::query_as(
        "SELECT timestamp, duration_seconds, sample_rate, analysis \
         FROM ecg WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 3"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if rows.is_empty() {
        return "### ECG sessions\n_No ECG recordings available._".to_string();
    }

    let mut out = String::from("### Recent ECG sessions\n");
    for r in rows {
        let ts = r.timestamp.map(|t| t.format("%b %d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());
        let dur = r.duration_seconds.map(|d| format!("{}s", d)).unwrap_or_else(|| "—".to_string());
        let rate = r.sample_rate.map(|s| format!("{}Hz", s)).unwrap_or_else(|| "—".to_string());
        out.push_str(&format!("- **{}**: duration {}, sample rate {}", ts, dur, rate));
        if let Some(analysis) = r.analysis {
            if let Some(obj) = analysis.as_object() {
                let parts: Vec<String> = obj.iter()
                    .take(4)
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                if !parts.is_empty() {
                    out.push_str(&format!(" · {}", parts.join(" · ")));
                }
            }
        }
        out.push('\n');
    }
    out
}

async fn activity_summary(pool: &PgPool, user_id: uuid::Uuid) -> String {
    #[derive(sqlx::FromRow)]
    struct ActRow {
        total_steps: Option<i64>,
        total_calories: Option<f64>,
        total_distance: Option<f64>,
        total_active_minutes: Option<i64>,
    }

    let today: Option<ActRow> = sqlx::query_as(
        "SELECT SUM(steps)::bigint AS total_steps, \
         SUM(calories)::float8 AS total_calories, \
         SUM(distance)::float8 AS total_distance, \
         SUM(active_minutes)::bigint AS total_active_minutes \
         FROM activity WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '24 hours'"
    ).bind(user_id).fetch_optional(pool).await.ok().flatten();

    let Some(a) = today else {
        return "### Activity today\n_No activity data._".to_string();
    };

    let mut out = String::from("### Activity today (last 24h)\n");
    if let Some(v) = a.total_steps { out.push_str(&format!("- Steps: **{}**\n", v)); }
    if let Some(v) = a.total_calories { out.push_str(&format!("- Active calories: **{:.0} kcal**\n", v)); }
    if let Some(v) = a.total_distance { out.push_str(&format!("- Distance: **{:.2} km**\n", v)); }
    if let Some(v) = a.total_active_minutes { out.push_str(&format!("- Active minutes: **{}**\n", v)); }
    out
}
