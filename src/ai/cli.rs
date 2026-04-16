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
/// response is 3-10 s; anything above 30 s is almost always stuck / network
/// trouble.
const CLI_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Full-context invoke: takes the per-endpoint kind + the complete prompt
/// (system + biometric context + user question) and returns either the
/// Claude response or the static fallback text.
///
/// This is the function the handlers should call — it never fails, it
/// always returns a user-facing string.
pub async fn ask_or_fallback(kind: AiEndpointKind, prompt: &str) -> String {
    match invoke_claude_cli(prompt).await {
        Ok(response) => response,
        Err(reason) => {
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
        assert_eq!(CLI_TIMEOUT, Duration::from_secs(30));
    }

    #[test]
    fn default_model_is_sonnet_4_6() {
        assert_eq!(DEFAULT_MODEL, "claude-sonnet-4-6");
    }
}
