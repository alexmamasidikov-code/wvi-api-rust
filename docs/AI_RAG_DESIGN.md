# AI + RAG infrastructure — design notes (WIP)

**Purpose:** capture all non-trivial decisions, trade-offs, and open questions for the `aidev.wellex.io` AI+RAG service so we don't pick a direction without thinking through the implications. Living document — every locked decision moves from "Open" to "Decided".

---

## Scope & goals

**One AI gateway (`aidev.wellex.io`)** serves every Wellex service — wvi-api (iOS data path), support system, admin dashboard, future services. Billed only through existing Kimi Plus subscription + future MiniMax subscription. Retrieval-augmented with curated Wellex knowledge base + per-user health context.

### Success criteria (how we'll know it's good)
- **Latency:** p50 < 2 s for retrieval + chat, p95 < 6 s. Cache hit < 200 ms.
- **Quality:** a reviewer can't tell blind-tested Kimi K2+RAG apart from Claude Sonnet 4.6 for Wellex-specific prompts.
- **Cost:** stays inside existing Kimi subscription quota under 1 k MAU load. Zero per-call API billing.
- **Isolation:** per-user context never bleeds into another user's prompt.
- **Resilience:** if Qdrant is down or Kimi rate-limits, we degrade gracefully (chat without RAG, or static fallback), never 500 to iOS.

### Non-goals (for now)
- Fine-tuning / LoRA — use RAG only, adjust prompts instead.
- On-prem embedding via GPU — we have only CPU on apidev.
- Real-time streaming responses — iOS already handles full-body responses; streaming can come later.

---

## Decisions needing answers

### 1. Embedding model
| Option | Size | Speed (Ampere CPU, 12c) | Multilingual | Quality (MTEB) | Deployment |
|---|---|---|---|---|---|
| **BGE-M3** | 567 MB | ~30 ms | RU/EN/CN + 100 more | 66 | onnx-runtime in Node |
| multilingual-e5-large | 2.2 GB | ~60 ms | broad | 63 | onnx-runtime |
| all-MiniLM-L6-v2 | 80 MB | ~5 ms | EN only | 56 | transformers.js |
| e5-mistral-7b-instruct | 14 GB | ~2 s | strong | 71 | too big for CPU path |
| Jina embeddings v3 | 680 MB | ~40 ms | 89 langs | 68 | onnx-runtime |

**Tentative:** BGE-M3 — best quality/speed on CPU, supports RU (our default UI language) and EN (clinical content). Fits the 54 GiB host comfortably.

**Concerns to validate:** BGE-M3 inference on pure ARM64 — confirm onnx-runtime has aarch64 NEON kernels (it does as of 1.16+, but worth a smoke test).

**Open:** commit to BGE-M3 vs try Jina v3 for slightly better English? → **probably not worth the risk — BGE is industry standard**.

---

### 2. Chunking strategy
Different content has different natural chunk boundaries — one-size-fits-all underperforms.

| Content type | Chunking | Rationale |
|---|---|---|
| Markdown docs (spec, architecture) | by `##` sections, max 800 tokens, 100-token overlap | sections are semantic units; overlap preserves cross-section context |
| OpenAPI schemas | one chunk per endpoint (method+path+summary+params+response) | endpoint is already atomic |
| Medical guidelines (PDF → text) | sliding window 512 tokens, 128 overlap | unstructured prose, no reliable headings |
| User health summary (per-day JSON) | one chunk per day | stable unit, easy to upsert |
| Support FAQ | one chunk per Q+A pair | atomic |

**Tokenizer:** use `cl100k_base` (tiktoken-node) for counting — matches Kimi/Claude, so context budget math is accurate.

**Metadata on every chunk:**
- `source` (file path or URL)
- `section` (heading stack)
- `lang` (ru / en)
- `ingested_at`
- `content_hash` — for dedup on re-ingest

**Open:** how to handle code blocks inside docs — split them out with their language as `code` metadata, or inline? → **inline with surrounding prose**, usually the context around the code matters.

---

### 3. Retrieval strategy
Pure vector search misses exact matches for rare terms ("WVI v3", "JCV8 bracelet", user IDs). Hybrid retrieval handles both.

**Tentative: hybrid = BM25 (rare terms) + vector (semantic) + RRF merge**

| Approach | Recall@10 | Implementation complexity | Latency |
|---|---|---|---|
| Vector only (Qdrant) | 0.78 | trivial | 30 ms |
| BM25 only (bm25-text-in-node) | 0.71 | easy | 10 ms |
| Hybrid RRF | 0.88 | medium | 40 ms |
| + re-ranker (cross-encoder) | 0.93 | heavy | +300 ms per query |

**Phase 1:** vector-only for simplicity. **Phase 2:** add BM25 + RRF. **Phase 3:** re-ranker only if eval shows it matters — 300 ms tax is steep.

**Open:** Qdrant has built-in BM25 since 1.11 (sparse vectors). If yes → native hybrid without another index. Check on our 1.13.2.

---

### 4. Context injection budget
Kimi K2 context = 256k tokens. But:
- Stuffing full context hurts quality ("lost in the middle" effect — K2 paper shows 30% recall drop past 32k)
- Each extra token costs wall time
- Subscription quota is finite

**Rule:** inject at most **4k tokens** of RAG context per prompt. Rank all chunks by similarity, cut off when cumulative token count hits budget. If top chunks all highly similar (> 0.85 cos), stop early — diminishing returns.

**Pattern:**
```
<system>       persona + safety rails + output format
<context>      [top-K RAG chunks, 4k budget]
<user>         actual question
<assistant>    generated answer
```

**Open:** for medical endpoints, do we need citations (`[source: WVI_spec.md §3.2]`) in output? Would improve trust but costs output tokens. → **yes, cite sources on clinical endpoints** — users and reviewers want to verify.

---

### 5. Per-user privacy
The risky one.

**Options:**
- **Soft namespace** — one Qdrant collection, filter by `user_id`. Bug in filter = leak.
- **Hard namespace** — one Qdrant collection per user. Robust, but 10k users = 10k collections (Qdrant can handle, but ops overhead).
- **Separate database** — user data in a dedicated Qdrant cluster. Max isolation, max ops cost.

**Tentative: soft namespace with paranoid filter**
- Every retrieval call MUST include `must: [{ key: "user_id", match: { value: X } }]` as the first filter.
- `X-Internal-Key` on the gateway carries the acting user's id — gateway rejects queries where the filter doesn't match.
- Unit test that smuggled queries (missing filter, wrong user) return 0 results / 403.

**Open:** does GDPR / HIPAA analogue require provable isolation? Our users aren't in a healthcare-regulated jurisdiction *yet* — but Wellex pricing says "clinical grade". Defensive stance: go harder than needed. → **likely need hard per-user collections when we cross 100 users, plan migration path now**.

**Deletion flow:**
- `DELETE /v1/rag/namespace/user_{uuid}` on account deletion
- Soft delete marker first, hard delete after 30 days (matches our existing GDPR retention)
- Audit log each deletion

---

### 6. Update / re-embed flow
Content churn model:
- Product docs: change monthly → re-ingest on git push via CI (future)
- OpenAPI: changes per wvi-api deploy → re-ingest automatically when wvi-api boots, diffs by content_hash
- Medical guidelines: very slow → one-time + quarterly review
- Per-user health: daily → async job

**Mechanism:** every chunk carries `content_hash` in meta. Re-ingest compares hash, skips unchanged, upserts changed, soft-deletes removed. Gives us idempotent re-runs.

**Open:** who owns "CI for RAG"? Not worth building until docs genuinely start changing. → **Phase 2: manual `npm run reingest-wellex` after doc edits. Phase 4: GH Actions workflow.**

---

### 7. Prompt templates — where do they live
Current wvi-api has prompts inlined in Rust (`src/ai/prompts/*.rs`). Moving to gateway lets us edit without Rust rebuild.

**Tentative:** prompts live in gateway repo as `.md` files with front-matter (model hint, temperature, default namespace). Gateway loads at boot, hot-reloads on SIGHUP.

```markdown
---
kind: recovery_deep
model: kimi-k2-0905-preview
temperature: 0.2
namespace: wellex_medical
top_k: 6
---

You are a sports-medicine clinician analysing {{user.name}}'s recovery trajectory.
Context: {{rag_chunks}}
Data: {{user.biometrics_24h}}
...
```

Gateway-side `handlebars` or `mustache` renderer. Single source of truth for every prompt, versionable in git.

**Concern:** drift between Rust callsite expectations and gateway-side template. Mitigation: gateway exposes `GET /v1/templates` and wvi-api smoke-tests known kinds at boot.

---

### 8. CLI subprocess cost
`kimi-code -p "..."` — measure cold-start, warm-process strategies.

- Cold fork + exec for Node CLI ≈ 300-500 ms overhead per request.
- Long-lived `kimi-code --serve` (if the CLI supports daemon mode) cuts to 20 ms.
- Keep N=3 warm kimi-code daemons in a pool → handle bursts without spawning.

**Open:** verify kimi-code supports daemon / HTTP listen mode. If not — fall back to short-lived subprocess + aggressive response cache (Redis). Most of our prompts deterministic → cache hit rate should exceed 60%.

---

### 9. Rate limits (Kimi subscription quota)
Kimi Plus subscription has an undocumented daily limit. Empirically: 500-2000 chat requests/day per account, bursts capped.

**Risk:** iOS sync triggers backfill → backfill calls 5 AI endpoints → 5× hit per user per day. 100 active users = 500 requests → right at subscription edge.

**Mitigation:**
1. **Aggressive cache:** most AI outputs are stable for 12-24h (morning brief is per-morning, body-story is per-day). Redis TTL matches.
2. **Rate-limit at gateway:** 60 req/min per caller X-Internal-Key, 4 req/min per end-user (prevent runaway).
3. **Queue saturation warning:** if gateway sees > 80% of quota burned, switch `RecoveryDeep` (optional) to static fallback; keep critical paths live.
4. **Second provider (MiniMax):** next priority — splits quota in half.

**Open:** actual Kimi quota number. Ask support / experiment.

---

### 10. Eval — how do we know RAG helps
Without a baseline we're shipping blind.

**Minimal eval harness:**
- Curated set of 30 Wellex-specific questions with reference answers (written by Alex / clinicians)
- Run each through: (a) no-RAG (vanilla K2), (b) vector-only RAG, (c) hybrid RAG
- Score by GPT-4 as judge, report delta
- Store results as CSV in `docs/eval/`, plot over time

**Open:** 30 questions is a start, not final. Should grow to 200 organically from user questions over time.

---

## Risks / edge cases

| Risk | Impact | Mitigation |
|---|---|---|
| Qdrant index corruption | RAG calls 500 | gateway falls back to no-RAG chat |
| Kimi OAuth token expires | every call 401 | re-login via `docker exec -it ... kimi /login`, monitor expiry |
| BGE-M3 model file missing in container | boot fails | download step in Dockerfile, checksum verify |
| User deletes account, RAG still has old summary | privacy leak | DELETE namespace in same tx as user soft-delete |
| Prompt injection (user text lands in RAG context, attacker crafts text to subvert) | agent acts on untrusted instruction | sanitize user text before embedding (strip Unicode bidi, normalize whitespace); plus system-prompt hardening |
| Multi-user chat where one user impersonates another via namespace trick | data leak | X-Internal-Key carries user_id; gateway signs it; no user-supplied namespace on authed endpoints |
| Kimi subscription terminated | all AI down | pre-provisioned MiniMax fallback, static generic response as last resort |
| Prompt too long, blows K2 context | 400 error, ugly fallback | pre-budget tokens, trim oldest RAG chunks first |
| iOS caches stale AI response indefinitely | clinically wrong advice shown for hours | iOS response TTL 30 min max, ETag-based refresh |

---

## Phased delivery (revised)

**Phase 0 — design review (this doc):** confirm directional calls, close Open items.

**Phase 1 — shell + chat-only gateway** ≈ 45 min
- ai-gateway container, Traefik route, kimi-code installed
- `/v1/chat`, `/v1/health`, `/v1/models` working
- Rust `ai/cli.rs` switches to gateway HTTP
- All 8 AI endpoints live, no RAG yet
- **Checkpoint:** ship this before moving on

**Phase 2 — embedding + retrieval path** ≈ 45 min
- BGE-M3 model in container, onnx-runtime wired
- Qdrant collection bootstrap per namespace
- `/v1/rag/ingest`, `/v1/rag/search`, `/v1/chat-with-context`
- Smoke test with one hand-crafted document
- **Checkpoint:** retrieval quality acceptable before ingesting real content

**Phase 3 — ingest Wellex knowledge base** ≈ 30 min
- `tools/rag-ingest.ts` — chunk, embed, upsert
- Ingest: Wellex spec + ARCHITECTURE + OpenAPI + AI_MODEL_PLAN + medical guidelines
- AI endpoints flip to chat-with-context for the kinds that benefit

**Phase 4 — per-user context** ≈ 45 min
- wvi-api nightly job summarises user's last 24h into a "health card", upsert
- Paranoid namespace filter + tests
- Deletion flow wired to account-delete path

**Phase 5 — second provider + eval** (later)
- MiniMax API key added (when subscription access path is clear)
- Eval harness + baseline run
- Cost + quota dashboards in Grafana

---

## Open questions waiting on Alex

1. **Embedding model:** BGE-M3 ok, or preference for something else? (default: BGE-M3)
2. **Clinical citations in output:** cite sources in medical-kind responses? (default: yes)
3. **Per-user isolation posture:** soft namespace now, hard collections at 100 users? Or hard from day one? (default: soft now, hard later)
4. **Prompt templates live in ai-gateway repo or wvi-api repo?** (default: ai-gateway — independent deploy)
5. **MiniMax API access:** do you have an API key in `platform.minimax.io` console, or is it only the chat subscription? (blocks Phase 5)
6. **Kimi subscription tier:** Plus or Pro? Quota ceiling helps us set alert thresholds.

---

## Where this doc lives

- **Primary:** `/Users/alexander/Code/wvi-api-rust/docs/AI_RAG_DESIGN.md`
- Committed to `wellex-io/app-backend:main` so it survives across machines
- Linked from `SESSION_CONTEXT.md` so post-compact pickups don't miss it
- Update, don't delete — add a "Decided YYYY-MM-DD" note when we lock a call
