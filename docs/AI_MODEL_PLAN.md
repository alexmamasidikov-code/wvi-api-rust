# AI Model Hosting Plan — apidev / Wellex dev cluster

**Context:** WVI API currently uses a local `claude` CLI (not available on the apidev server), so every AI endpoint is returning its static fallback. We want to move to one of two target models the user named — **Kimi K2** (Moonshot, 1T-param MoE) or **MiniMax M2** (~230B-param MoE) — without compromising latency or the clinical quality the iOS Mind / AI Insights tabs depend on.

This document captures the hardware reality, evaluates three deployment paths, and recommends the one we should ship first.

---

## Hardware reality (apidev / `wellex_dev`)

```
Host      : novel-snipe (88.216.62.20, Wellex dev cluster)
CPU       : 12 × Ampere-1a (ARM64 v8.2-A) — server-grade but general-purpose
RAM       : 54 GiB (49 GiB free under current load)
Disk      : 290 GiB NVMe (231 GiB free)
GPU       : NONE
OS        : Ubuntu 24.04 LTS (arm64)
Container : Docker 27 on `wellex-net`
```

**This is decisive.** Without an NVIDIA or AMD accelerator we have no path to running either Kimi K2 or MiniMax M2 in their native form — the weights alone (even at INT4) exceed host RAM by an order of magnitude:

| Model | Params | Active per token | Weights FP16 | Weights Q4_K_M | Runnable on 54 GiB RAM? |
|---|---|---|---|---|---|
| **Kimi K2** | 1 T (MoE) | 32 B | ≈ 2 000 GiB | ≈ 500 GiB | ❌ nowhere close |
| **MiniMax M2** | 230 B (MoE) | 16 B | ≈ 460 GiB | ≈ 120 GiB | ❌ |
| Qwen 2.5 32 B | 32 B | 32 B | ≈ 64 GiB | ≈ 18 GiB | ✅ (comfortable) |
| Llama 3.1 8 B | 8 B | 8 B | ≈ 16 GiB | ≈ 4.7 GiB | ✅ (trivial) |

ARM64 also means we cannot use CUDA builds — any local runtime must target `aarch64` + NEON (llama.cpp and Ollama both support this; MLX / TensorRT / vLLM do not).

---

## Three paths

### A. Hosted API (recommended for first iteration)

Call Kimi K2 / MiniMax M2 over their respective cloud APIs. Our backend stays stateless; the iOS client never changes.

**Endpoints**
- Kimi K2 — `https://api.moonshot.cn/v1/chat/completions` (OpenAI-compatible). Also available via OpenRouter (`moonshotai/kimi-k2`) if we want one provider abstraction over both.
- MiniMax M2 — `https://api.minimax.chat/v1/text/chatcompletion_v2` (native) or via OpenRouter (`minimax/minimax-m2`).

**Integration**
Replace `ai/cli.rs::invoke()` (currently shells out to the `claude` CLI) with a `reqwest` client that posts to whichever provider is configured. The five canonical endpoint kinds (`DailyBrief`, `EveningReview`, `RecoveryDeep`, `WeeklyDeep`, `EcgInterpret`) become a trivial routing table over provider + model slug.

```rust
// src/ai/cli.rs — direct drop-in replacement
pub async fn invoke(prompt: AiPrompt, endpoint: EndpointKind) -> Result<String> {
    let provider = AppConfig::ai_provider();          // "moonshot" | "minimax" | "openrouter"
    let model = endpoint.model_slug_for(provider);    // e.g. "kimi-k2-0425" for RecoveryDeep
    let body = openai_chat_body(prompt, model);
    let resp: OAIResponse = http_client()
        .post(provider.endpoint())
        .bearer_auth(AppConfig::ai_api_key())
        .json(&body)
        .send().await?
        .error_for_status()?
        .json().await?;
    Ok(resp.choices[0].message.content.clone())
}
```

**Cache unchanged** — the existing `prompt.hash → response` Redis layer stays intact; first-call latency absorbs the 300-700 ms API round-trip, cache hits stay < 10 ms.

**Pros**
- Zero hardware concern. We get the full Kimi K2 / MiniMax M2 model quality with no quantization tax.
- Quick to ship — 1-day PR.
- Cost is linear with usage and trivially capped per-user via Redis rate limiter we already own.

**Cons**
- Per-call cost. Kimi K2 is roughly $0.60 / M input + $2.50 / M output, MiniMax M2 about $0.40 / $2.00. For our tier-1 report endpoints (≈ 8 KB prompts, 1 KB completions) this is ≈ $0.007 per call — ≈ $7 per 1 k users per day.
- External dependency. If Moonshot is degraded, so is our AI pane — but the iOS UI already renders the static fallback gracefully.
- Health data leaves our network. We strip identifiers before sending (already true for the existing Claude CLI path), so this is a policy choice, not a compliance block — but worth naming explicitly.

### B. Local small model (complement, not replacement)

Run a 7-32 B model on the apidev host via **Ollama** (simplest) or **llama.cpp** (leanest).

Best-fit candidates for Ampere + 54 GiB RAM:

| Model | Q4_K_M size | Inference speed (12-core Ampere) | Quality vs Claude Sonnet 4.6 |
|---|---|---|---|
| Qwen 2.5 14B Instruct | 8.5 GiB | ≈ 10-12 tok/s | ≈ 80 % |
| Qwen 2.5 32B Instruct | 19 GiB | ≈ 4-5 tok/s | ≈ 92 % |
| Llama 3.1 8B Instruct | 4.7 GiB | ≈ 18-20 tok/s | ≈ 70 % |
| DeepSeek R1 Distill 14B | 9 GiB | ≈ 9-10 tok/s | ≈ 85 % (best reasoning) |

```bash
# Ollama bring-up
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen2.5:14b-instruct-q4_K_M
ollama serve                                # listens on 127.0.0.1:11434

# Test
curl http://localhost:11434/api/chat \
     -d '{"model":"qwen2.5:14b-instruct-q4_K_M","messages":[...]}'
```

Add an `ollama` service to `/srv/wellex/docker-compose.yml`, expose it on `wellex-net` (no public ingress), point `wvi-api` at it via `OLLAMA_URL=http://ollama:11434`.

**Pros**
- Zero marginal cost per call.
- Stays on our network — no data egress.
- Predictable latency.

**Cons**
- Quality gap vs. Kimi K2 / MiniMax M2 is material for the hardest endpoints (`RecoveryDeep`, `EcgInterpret`, `WeeklyDeep`). A 14-32 B dense model on CPU is not in the same league as a 1 T MoE.
- 4-5 tok/s means a 2 kB completion takes 15-20 seconds, which is too slow for the Morning Brief panel (users wait).
- No Kimi K2 / MiniMax M2 available in this size class — we'd be shipping a different model family.

### C. Hybrid (recommended long-term)

Route by endpoint importance:

| Endpoint | Target | Why |
|---|---|---|
| `MorningBrief` | local Qwen 14 B | 200-400 tok output, fast enough, low-stakes nudge |
| `EveningReview` | local Qwen 14 B | same |
| `RecoveryDeep` | **Kimi K2 / MiniMax M2 API** | clinician-style output; quality matters |
| `WeeklyDeep` | **Kimi K2 / MiniMax M2 API** | same |
| `EcgInterpret` | **Kimi K2 / MiniMax M2 API** | clinical |
| `FullAnalysis` | **Kimi K2 / MiniMax M2 API** | marquee feature |

This is the path for month 2-3: we get the cost win on the chatty endpoints and keep the premium endpoints on the premium model.

---

## Recommendation

**Phase 1 (this sprint):** go with **path A — hosted API**, pick one provider, instrument per-call cost metrics.

- Pick Kimi K2 (Moonshot) as the default — their pricing is slightly higher than MiniMax M2 but the knowledge cut-off is fresher and the OpenAI-compatible API is battle-tested via OpenRouter.
- Store `AI_PROVIDER=moonshot`, `AI_API_KEY=<...>`, `AI_MODEL=kimi-k2-0425` in `/srv/wellex/.env`; wire into `docker-compose.wvi.yml`.
- Replace the `claude` CLI shell-out in `src/ai/cli.rs` with an OpenAI-compatible `reqwest` client.
- Emit Prometheus counters for `ai_calls_total{endpoint, provider}` and `ai_tokens_total{direction}`.
- Ship behind a kill-switch flag: if calls fail, fall back to the existing static response (no worse than today).

**Phase 2:** add **path B — local Qwen 14 B via Ollama** as a second provider and flip Morning/Evening briefs over to it behind a per-endpoint routing table (path C).

**Phase 3:** revisit when / if the apidev cluster grows a GPU box (single H100 would unlock local MiniMax M2 with quantization; two H200s needed for real Kimi K2 inference).

---

## Implementation checklist

- [ ] Create `src/ai/providers/` module with one file per provider (`moonshot.rs`, `minimax.rs`, `ollama.rs`, `static_fallback.rs`) behind a common `trait AiProvider`.
- [ ] Thread `AppConfig::ai_provider()` through `cli::invoke()` so tests can swap providers without env vars.
- [ ] Add `OLLAMA_HEALTHY` Prometheus gauge and a `/api/v1/health/ai` probe so Grafana can page when local inference falls over.
- [ ] Update `docs/openapi.yaml` — no request-shape change, but add `x-ai-provider` response header so iOS can display a discreet chip when the model served was the fallback.
- [ ] Budget guard: daily token cap per user stored in Redis; 429 when exceeded. We already have the rate-limit middleware; adding a token-level counter is ~30 lines.
- [ ] `AGENTS.md` appendix: document the provider configuration matrix for on-call handoff.

Once Phase 1 ships, the iOS `HomeScreen` AI Insights pane will start returning real Kimi-K2-grade content with no iOS-side code change. The user pays per-call, but at expected MAU for the dev cohort the daily bill is under $5.
