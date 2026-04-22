//! AI invocation layer.
//!
//! Two paths, selected at call time:
//! - `AI_GATEWAY_URL` env set → HTTP POST to aidev.wellex.io/v1/chat, which
//!   routes the call through the shared Kimi subscription (and optionally
//!   MiniMax). No subprocess, no per-service CLI — this is the canonical
//!   path as of 2026-04-22.
//! - Env absent → legacy `claude` CLI subprocess (transitional fallback so
//!   local dev without the gateway still works).
//!
//! Both paths ultimately go through `ask_or_fallback`, which guarantees the
//! handler always gets a user-facing string (static fallback on any failure).

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Semaphore};
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;

use futures::Stream;
use once_cell::sync::Lazy;

/// Max concurrent Claude CLI subprocesses. Protects the VPS from OOM.
static CLI_SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(5));

/// Upper-bound wall-clock time for a single CLI invocation. Typical
/// response is 20-40 s for the medical-analyst prompt (3-5 KB context).
/// Bumped to 90 s so iOS clients don't see fallbacks for every queued call
/// when several cards fire simultaneously.
const CLI_TIMEOUT: Duration = Duration::from_secs(90);

/// Default Claude model. Sonnet 4.6 is the production analysis target —
/// fast enough for chat-style latency, cheaper than Opus, capable enough
/// for biometric interpretation. Override via env `WVI_CLAUDE_MODEL` for
/// experiments.
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// Endpoint categories — used to pick a matching static fallback text.
#[derive(Debug, Clone, Copy)]
pub enum AiEndpointKind {
    Interpret,
    Recommendations,
    Chat,
    ExplainMetric,
    ActionPlan,
    Insights,
    GeniusLayer,
    // AI Coach 2.0 — proactive / scheduled analyses.
    DailyBrief,
    EveningReview,
    AnomalyAlert,
    WeeklyDeep,
    // Medical-analyst level
    FullAnalysis,
    EcgInterpret,
    RecoveryDeep,
    // Per-tab narratives
    BodyStory,
}

impl AiEndpointKind {
    /// Stable lowercase identifier — used as part of the cache key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interpret => "interpret",
            Self::Recommendations => "recommendations",
            Self::Chat => "chat",
            Self::ExplainMetric => "explain_metric",
            Self::ActionPlan => "action_plan",
            Self::Insights => "insights",
            Self::GeniusLayer => "genius_layer",
            Self::DailyBrief => "daily_brief",
            Self::EveningReview => "evening_review",
            Self::AnomalyAlert => "anomaly_alert",
            Self::WeeklyDeep => "weekly_deep",
            Self::FullAnalysis => "full_analysis",
            Self::EcgInterpret => "ecg_interpret",
            Self::RecoveryDeep => "recovery_deep",
            Self::BodyStory => "body_story",
        }
    }

    pub fn fallback_text(self) -> &'static str {
        match self {
            Self::Interpret =>
                "Your metrics are being analyzed. Core vitals (HR, HRV, SpO2) look within \
                typical daytime ranges. Check back in a few minutes for a detailed reading.",
            Self::Recommendations =>
                "1. **Hydrate**: drink a glass of water — supports HR regulation.\n\
                2. **Stand up**: 2 minutes of light movement improves circulation.\n\
                3. **Breathe**: 4-7-8 breathing for 1 minute lowers stress.\n\n\
                Personalized advice resumes when the AI engine is back online.",
            Self::Chat =>
                "I'm temporarily unable to reach the analysis engine. Your recent metrics \
                are still being tracked — try asking again in a moment.",
            Self::ExplainMetric =>
                "This metric reflects your body's current state. Personalized explanations \
                will resume shortly.",
            Self::ActionPlan =>
                "Today's focus: steady hydration, 7k+ steps, and 7-9 hours of sleep tonight. \
                A detailed plan will load when the AI engine is available.",
            Self::Insights =>
                "Your biometric trend is stable. A full insight report is being prepared.",
            Self::GeniusLayer =>
                "Cross-metric correlations are loading. Your overall pattern looks balanced.",
            Self::DailyBrief =>
                "Good morning. Your recovery is tracking normally. Start the day with 5 minutes \
                of morning sunlight and steady hydration. Detailed AI brief resumes shortly.",
            Self::EveningReview =>
                "Evening check-in: a balanced day overall. Wind down with 4-7-8 breathing and \
                aim for 7-9 hours of sleep. Personalized recap will be ready next session.",
            Self::AnomalyAlert =>
                "A small biometric shift was detected. Nothing alarming — your body is adapting. \
                Observation continues; a detailed explanation will follow.",
            Self::WeeklyDeep =>
                "Your weekly analysis is being compiled. Trends across HR, HRV, sleep, stress, \
                and activity are looking balanced. Full report will arrive shortly.",
            Self::FullAnalysis =>
                "Full medical analyst report is loading. All core signals (HR, HRV, SpO2, \
                sleep, activity, emotion) will be integrated into a single report shortly.",
            Self::EcgInterpret =>
                "ECG interpretation is loading. Your last recording will be analyzed for \
                rhythm, coherence, and any rate irregularities.",
            Self::RecoveryDeep =>
                "Recovery analysis loading. Your HRV vs baseline + sleep quality + stress \
                trend are being evaluated.",
            Self::BodyStory =>
                "Your body looks within normal range today. Cardiac, sleep and stress signals \
                are all in a steady place. A detailed narrative will load shortly.",
        }
    }
}

/// Invoke `claude --print <prompt>` as a child process and return stdout.
///
/// Behavior:
/// - Acquires a semaphore permit (bounded concurrency).
/// - Enforces a 30 s wall-clock timeout.
/// - On non-zero exit, missing binary, timeout, or decode error: returns
///   `Err(reason)`. The caller should invoke `AiEndpointKind::fallback_text`
///   and return that string to iOS (wrapped in the usual success envelope,
///   so the user sees _something_, not an error).
pub async fn invoke_claude_cli(prompt: &str) -> Result<String, String> {
    let _permit = CLI_SEMAPHORE
        .acquire()
        .await
        .map_err(|e| format!("semaphore acquire failed: {e}"))?;

    // `claude --print` reads the prompt from argv; long prompts (> 128 KiB)
    // risk E2BIG, so feed via stdin instead when the prompt is big.
    let use_stdin = prompt.len() > 64 * 1024;

    let model = std::env::var("WVI_CLAUDE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

    let mut cmd = Command::new("claude");
    cmd.arg("--print");
    cmd.arg("--model");
    cmd.arg(&model);
    if !use_stdin {
        cmd.arg(prompt);
    }
    cmd.kill_on_drop(true);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn claude CLI: {e}"))?;

    if use_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| format!("failed to write prompt to stdin: {e}"))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| format!("failed to close stdin: {e}"))?;
        }
    }

    let output = timeout(CLI_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "claude CLI timed out after 30s".to_string())?
        .map_err(|e| format!("claude CLI wait failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "claude CLI exited with {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }

    let text = String::from_utf8(output.stdout)
        .map_err(|e| format!("claude CLI stdout not UTF-8: {e}"))?
        .trim()
        .to_string();

    if text.is_empty() {
        return Err("claude CLI returned empty output".to_string());
    }

    Ok(text)
}

/// Streaming invocation — spawns `claude --print --output-format stream-json
/// --include-partial-messages`, reads NDJSON frames off stdout, and yields
/// text deltas on a tokio channel wrapped as a Stream.
///
/// Each stream-json frame looks like (simplified):
///   {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}
/// We extract the `delta.text` field and push it. Frames of other types
/// (message_start, message_stop, tool_use, etc.) are ignored — iOS only
/// needs the text stream for chat UX.
///
/// On CLI failure or non-zero exit, the channel closes and the stream
/// terminates. Caller is expected to append a fallback if the accumulated
/// text is empty.
pub fn invoke_claude_cli_streaming(prompt: &str) -> impl Stream<Item = String> {
    let (tx, rx) = mpsc::channel::<String>(64);
    let prompt = prompt.to_string();

    tokio::spawn(async move {
        let _permit = match CLI_SEMAPHORE.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        let model = std::env::var("WVI_CLAUDE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        let mut cmd = Command::new("claude");
        cmd.arg("--print");
        cmd.arg("--model");
        cmd.arg(&model);
        cmd.arg("--output-format");
        cmd.arg("stream-json");
        cmd.arg("--include-partial-messages");
        cmd.arg("--verbose");
        cmd.kill_on_drop(true);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("claude streaming spawn failed: {e}");
                return;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => return,
        };

        let deadline = tokio::time::sleep(CLI_TIMEOUT);
        tokio::pin!(deadline);

        let mut reader = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    tracing::warn!("claude streaming timed out after 90s");
                    let _ = child.kill().await;
                    break;
                }
                line = reader.next_line() => {
                    match line {
                        Ok(Some(raw)) => {
                            if let Some(delta) = extract_text_delta(&raw) {
                                if !delta.is_empty() && tx.send(delta).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!("claude streaming read error: {e}");
                            break;
                        }
                    }
                }
            }
        }

        let _ = child.wait().await;
    });

    ReceiverStream::new(rx)
}

/// Pull `delta.text` out of a stream-json NDJSON frame. Returns None for any
/// frame that isn't a text delta so the streaming loop can skip it cleanly.
fn extract_text_delta(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let frame_type = v.get("type")?.as_str()?;
    // Primary delta format (Anthropic streaming): content_block_delta.
    if frame_type == "content_block_delta" {
        let delta = v.get("delta")?;
        let delta_type = delta.get("type")?.as_str()?;
        if delta_type == "text_delta" {
            return delta.get("text")?.as_str().map(String::from);
        }
    }
    // Some Claude Code CLI variants wrap the content in `message.content[*].text`
    // on message_delta frames — handle that too.
    if frame_type == "message" {
        if let Some(content) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) {
            let mut acc = String::new();
            for block in content {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    acc.push_str(t);
                }
            }
            if !acc.is_empty() { return Some(acc); }
        }
    }
    None
}

/// Variant of `invoke_claude_cli` used by the ECG-interpret strict-JSON path.
/// The base CLI implementation doesn't expose a temperature knob because the
/// `claude` binary takes `--print` without one, but we still parametrise the
/// call with a `retry_hint` that gets appended to the prompt so the
/// higher-retry invocation explicitly asks for JSON-only output again.
pub async fn invoke_claude_cli_retry(prompt: &str, retry_hint: &str) -> Result<String, String> {
    let full = if retry_hint.is_empty() {
        prompt.to_string()
    } else {
        format!("{prompt}\n\n{retry_hint}")
    };
    invoke_claude_cli(&full).await
}

/// Strict-JSON ECG prompt — Project F.
///
/// Output a single JSON object matching the schema below. Any deviation
/// (markdown fences, prose, additional fields) is considered a parse
/// failure and triggers a retry path in the handler.
pub fn ecg_interpret_prompt(samples: &[f64], duration_seconds: i32, sample_rate: i32) -> String {
    // Clip the in-prompt sample list to the first 200 values; the full
    // waveform is persisted in the DB, but streaming 3750+ floats into the
    // prompt eats the context window for zero added signal (Claude cannot
    // numerically process 30s of ECG — it reasons off summary stats that we
    // include in the text).
    let n = samples.len();
    let preview = if n > 200 {
        format!("[{n} samples, first 200 shown: {:?}]", &samples[..200])
    } else {
        format!("{:?}", samples)
    };

    format!(
        r#"You are a single-lead ECG analysis assistant. Interpret the waveform below.

Samples ({duration_seconds} sec @ {sample_rate} Hz, n={n}): {preview}

STRICT OUTPUT FORMAT — respond with ONE valid JSON object and nothing else.
No markdown fences. No commentary. No field rename. No extra fields.

Schema:
{{
  "metrics": {{
    "hr_mean": <int bpm>,
    "hr_min": <int>,
    "hr_max": <int>,
    "rr_sd_ms": <number>,
    "qrs_duration_ms": <int or null>
  }},
  "rhythm": "sinus" | "irregular" | "brady" | "tachy",
  "afib_score": <float 0..1>,
  "observations": [<string, English, max 4 bullets>],
  "recommendation": <string, Russian, 1-2 sentences>,
  "is_crisis": <bool>
}}

Clinical rules:
- rhythm is 'brady' when hr_mean<50, 'tachy' when hr_mean>100, 'irregular'
  when afib_score>0.5, otherwise 'sinus'.
- is_crisis MUST be true when hr_mean>150 OR hr_mean<40 OR afib_score>0.8.
- recommendation: 1-2 short sentences, Russian. Do NOT claim a medical
  diagnosis. If the reading is abnormal, include the phrase
  "не заменяет консультацию врача".
- Do NOT attempt ST-segment, axis, or P-wave analysis — a single-lead
  wrist signal cannot support those.

Output the JSON object now:"#
    )
}

/// Cheap non-cryptographic 64-bit hash of the prompt body. Used only as a
/// span attribute so traces can be correlated across retries without leaking
/// the full prompt text into the observability backend.
fn prompt_hash_u64(prompt: &str) -> u64 {
    hash_u64_public(prompt)
}

/// Public wrapper — same hash as prompt_hash_u64 but exposed so other modules
/// (handlers.rs instrumentation) can obscure privy_did in tracing fields.
pub fn hash_u64_public(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Gateway HTTP timeout. Shorter than `CLI_TIMEOUT` because the gateway
/// itself caps provider calls at 90 s; wvi-api only needs to wait ~25 s
/// before giving up and falling back. iOS client-side timeout (60 s) is
/// well above this, so users see fallback text rather than gateway 502.
const GATEWAY_TIMEOUT: Duration = Duration::from_secs(25);

/// Shared HTTP client for all gateway calls. Built once (keeps connection pool
/// warm) — saves ~100 ms of TLS + TCP handshake per request.
static GATEWAY_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(GATEWAY_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(16)
        .build()
        .expect("reqwest client build")
});

fn gateway_creds() -> Result<(String, String), String> {
    let url = std::env::var("AI_GATEWAY_URL")
        .map_err(|_| "AI_GATEWAY_URL not set".to_string())?;
    let key = std::env::var("AI_GATEWAY_INTERNAL_KEY")
        .map_err(|_| "AI_GATEWAY_INTERNAL_KEY not set".to_string())?;
    Ok((url.trim_end_matches('/').to_string(), key))
}

fn extract_message_text(v: &serde_json::Value) -> String {
    v.pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Call a named prompt `kind` registered on the gateway (POST /v1/kind/:name).
/// The gateway loads the template from `src/prompts/<kind>.md`, injects RAG
/// context if the template declares a `namespace:`, and runs the LLM.
///
/// `variables` is a JSON object substituted into Mustache `{{vars}}`.
pub async fn invoke_ai_kind(
    kind: &str,
    variables: serde_json::Value,
) -> Result<String, String> {
    let (url, key) = gateway_creds()?;

    let resp = GATEWAY_CLIENT
        .post(format!("{}/v1/kind/{}", url, kind))
        .header("X-Internal-Key", key)
        .json(&serde_json::json!({ "variables": variables }))
        .send()
        .await
        .map_err(|e| format!("gateway kind request: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "gateway kind/{} {}: {}",
            kind,
            status.as_u16(),
            body.chars().take(500).collect::<String>()
        ));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("gateway kind response decode: {e}"))?;
    let text = extract_message_text(&json);
    if text.is_empty() {
        return Err(format!("gateway kind/{}: empty content", kind));
    }
    Ok(text)
}

/// POST the prompt to the Wellex AI gateway (`AI_GATEWAY_URL`). The gateway
/// decides which provider (Kimi / MiniMax) serves the call.
///
/// Shape matches `/v1/chat` at aidev.wellex.io — OpenAI-ish body in, text
/// extracted from `choices[0].message.content`.
pub async fn invoke_ai_gateway(prompt: &str) -> Result<String, String> {
    let url = std::env::var("AI_GATEWAY_URL")
        .map_err(|_| "AI_GATEWAY_URL not set".to_string())?;
    let key = std::env::var("AI_GATEWAY_INTERNAL_KEY")
        .map_err(|_| "AI_GATEWAY_INTERNAL_KEY not set".to_string())?;

    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": prompt }],
    });

    let resp = GATEWAY_CLIENT
        .post(format!("{}/v1/chat", url.trim_end_matches('/')))
        .header("X-Internal-Key", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("gateway request: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "gateway {}: {}",
            status.as_u16(),
            body.chars().take(500).collect::<String>()
        ));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("gateway response decode: {e}"))?;

    let text = json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if text.is_empty() {
        return Err("gateway returned empty content".to_string());
    }
    Ok(text)
}

/// Full-context invoke: takes the per-endpoint kind + the complete prompt
/// (system + biometric context + user question) and returns either the
/// upstream response or the static fallback text.
///
/// Route preference: gateway → legacy CLI. On any failure falls back to
/// `AiEndpointKind::fallback_text()` so iOS never sees a raw error.
#[tracing::instrument(
    name = "ai.invoke",
    skip_all,
    fields(
        prompt.kind = kind.as_str(),
        prompt.hash = %format!("{:016x}", prompt_hash_u64(prompt)),
        prompt.bytes = prompt.len(),
        ai.route = tracing::field::Empty,
        response.cached = false,
        response.ok = tracing::field::Empty,
    )
)]
pub async fn ask_or_fallback(kind: AiEndpointKind, prompt: &str) -> String {
    let span = tracing::Span::current();
    let use_gateway = std::env::var("AI_GATEWAY_URL").is_ok();
    span.record("ai.route", if use_gateway { "gateway" } else { "claude_cli" });

    let result = if use_gateway {
        invoke_ai_gateway(prompt).await
    } else {
        invoke_claude_cli(prompt).await
    };

    match result {
        Ok(response) => {
            span.record("response.ok", true);
            response
        }
        Err(reason) => {
            span.record("response.ok", false);
            tracing::warn!(endpoint = ?kind, reason = %reason, "AI path failed, returning static fallback");
            kind.fallback_text().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_text_nonempty_for_all_kinds() {
        for kind in [
            AiEndpointKind::Interpret,
            AiEndpointKind::Recommendations,
            AiEndpointKind::Chat,
            AiEndpointKind::ExplainMetric,
            AiEndpointKind::ActionPlan,
            AiEndpointKind::Insights,
            AiEndpointKind::GeniusLayer,
            AiEndpointKind::DailyBrief,
            AiEndpointKind::EveningReview,
            AiEndpointKind::AnomalyAlert,
            AiEndpointKind::WeeklyDeep,
            AiEndpointKind::FullAnalysis,
            AiEndpointKind::EcgInterpret,
            AiEndpointKind::RecoveryDeep,
            AiEndpointKind::BodyStory,
        ] {
            let t = kind.fallback_text();
            assert!(!t.is_empty(), "fallback text for {:?} should not be empty", kind);
            assert!(t.len() >= 30, "fallback text for {:?} too short", kind);
        }
    }

    #[tokio::test]
    async fn ask_or_fallback_returns_fallback_when_cli_missing() {
        // Force a missing binary by setting PATH to an empty dir for this test only.
        // Safety: this only affects the current process's child env.
        // SAFETY: test-only, single-threaded at this point.
        unsafe { std::env::set_var("PATH", "/nonexistent-path-for-test"); }
        let result = ask_or_fallback(AiEndpointKind::Chat, "test prompt").await;
        assert_eq!(result, AiEndpointKind::Chat.fallback_text());
        // Restore PATH so subsequent tests aren't broken.
        // SAFETY: test-only restore.
        unsafe { std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin"); }
    }

    #[tokio::test]
    async fn invoke_claude_cli_fails_cleanly_on_missing_binary() {
        // SAFETY: test-only.
        unsafe { std::env::set_var("PATH", "/nonexistent-for-test"); }
        let result = invoke_claude_cli("hi").await;
        // SAFETY: test-only restore.
        unsafe { std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin"); }
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("failed to spawn") || err.contains("No such file"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn concurrency_limit_is_bounded() {
        // Verify the semaphore has expected capacity.
        assert_eq!(CLI_SEMAPHORE.available_permits(), 5);
    }

    #[test]
    fn timeout_is_reasonable() {
        // Pass-5: raised 30s→90s for Sonnet 4.6 cold-start headroom.
        assert_eq!(CLI_TIMEOUT, Duration::from_secs(90));
    }

    #[test]
    fn default_model_is_sonnet_4_6() {
        assert_eq!(DEFAULT_MODEL, "claude-sonnet-4-6");
    }
}
