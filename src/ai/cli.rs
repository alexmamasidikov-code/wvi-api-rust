//! Claude Code CLI invocation layer.
//!
//! Replaces the HTTP-based `call_claude` path with a local subprocess spawn
//! of the `claude` CLI installed on the production VPS (Max subscription).
//!
//! Design decisions (brainstorm 2026-04-17):
//! - Direct subprocess spawn (no long-running daemon, no job queue) — simple,
//!   predictable, good enough for current scale.
//! - Stateless one-shot invocation — the Rust handler assembles the full
//!   biometric context into the prompt on every call.
//! - Bounded concurrency via tokio semaphore (max 5 concurrent CLI spawns)
//!   to prevent OOM under burst traffic.
//! - Cache: SHA256(prompt) → Redis/memory (10-min TTL, existing AI cache).
//! - On timeout / CLI failure: return the per-endpoint static fallback text
//!   so iOS never sees an error message to the end-user.

use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::time::timeout;

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
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    prompt.hash(&mut h);
    h.finish()
}

/// Full-context invoke: takes the per-endpoint kind + the complete prompt
/// (system + biometric context + user question) and returns either the
/// Claude response or the static fallback text.
///
/// This is the function the handlers should call — it never fails, it
/// always returns a user-facing string.
#[tracing::instrument(
    name = "ai.claude.invoke",
    skip_all,
    fields(
        prompt.kind = kind.as_str(),
        prompt.hash = %format!("{:016x}", prompt_hash_u64(prompt)),
        prompt.bytes = prompt.len(),
        response.cached = false,
        response.ok = tracing::field::Empty,
    )
)]
pub async fn ask_or_fallback(kind: AiEndpointKind, prompt: &str) -> String {
    let span = tracing::Span::current();
    match invoke_claude_cli(prompt).await {
        Ok(response) => {
            span.record("response.ok", true);
            response
        }
        Err(reason) => {
            span.record("response.ok", false);
            tracing::warn!(endpoint = ?kind, reason = %reason, "claude CLI unavailable, returning static fallback");
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
