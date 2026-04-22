# Wellex API

**Production-grade Rust backend for the Wellex health platform.**

High-throughput biometric ingestion, the WVI (Wellness Vitality Index) scoring engine, an 18-component emotion fuzzy-logic classifier, and Claude-powered clinical insights — all served over a single Axum binary.

---

## Why it's fast

```
127 endpoints · 18 emotion classifiers · 10-metric WVI v3 · 6ms p50 latency
```

- **Stateless Axum handlers** — every request is a pure function over `(AuthUser, PgPool)`. No in-process state, no locks.
- **SQLx compile-time query verification** — queries validated against live schema at build time; runtime surprises are eliminated.
- **Aggregated at the DB** — `granularity=daily` / 7-day / monthly trends compute inside Postgres (`GROUP BY` + `percentile_cont`), not in the app. The app layer just shapes JSON.
- **Tokio spawn fan-out** for long-running tasks (WVI backfill scheduler, AI prompt warm-up, Kafka consumer) so the main request path never blocks.
- **Docker Compose production stack** — Postgres 16 / Redis 7 / Kafka / Zookeeper / Prometheus / Grafana / Loki behind Traefik TLS.

---

## Tech Stack

| Layer | Component |
|---|---|
| Language | **Rust 1.94+**, edition 2021 |
| HTTP | **Axum 0.8** + Tower middleware |
| Runtime | **Tokio** (multi-thread, work-stealing) |
| DB | **PostgreSQL 16** + SQLx (compile-time query verification) |
| Cache | **Redis 7** (alpine) |
| Streaming | **Kafka + Zookeeper** for biometric ingestion firehose |
| Auth | **Privy JWT (ES256)** via JWKS + Argon2 fallback |
| AI | **Claude Sonnet 4.6 / Opus 4.7** via local `claude` CLI (warm-cached) |
| Observability | **Sentry + Prometheus + Grafana + Loki** |
| Reverse proxy | **Traefik v3** with automatic Let's Encrypt |
| OpenAPI | **utoipa** → `/api/docs` |

---

## Architecture at a glance

```
                    ┌─────────────────┐
   iOS app ───JWT──►│   Traefik TLS   │
   Watch app ──────►│   (api.wvi...)  │
                    └────────┬────────┘
                             │ 8091
                    ┌────────▼────────┐
                    │   Axum router   │◄── 127 handlers
                    │   + AuthUser    │    across 40 modules
                    └───┬──────┬──────┘
         ┌──────────────┤      ├────────────────┐
         │              │      │                │
    ┌────▼───┐    ┌─────▼──┐  ┌▼────────┐  ┌────▼───┐
    │ Postgres│    │ Redis  │  │ Kafka   │  │ Claude │
    │ + SQLx  │    │ cache  │  │ stream  │  │ CLI    │
    └─────────┘    └────────┘  └─────────┘  └────────┘
```

### Module map

```
src/
├── auth/            Privy JWT middleware + sync
├── users/           Profile, persona, norms calibration
├── biometrics/      HR, HRV, SpO2, temp, sleep, PPI, ECG, BP, activity,
│                    stress, breathing, recovery, coherence, cardio-summary,
│                    bio-age-detail, hrv-detail, recovery-detail,
│                    calories-detail, vo2-detail
├── wvi/             10-metric WVI scoring + v3 (18-component personalised)
│                    /current /history /streak /trends /predict /breakdown ...
├── emotions/        Primary emotion classifier + v2 intraday
│                    /current /history /distribution /heatmap /triggers
│                    /today-hourly /transitions /wellbeing
├── ai/              Claude prompt orchestration + CLI warm-cache
├── stress/v2/       Sources breakdown, intraday micro-pulse detection
├── intraday/        1-minute rollups across all biometrics
├── sleep/           Multi-night aggregation + phase analysis
├── activities/      Workout ingestion, ACWR training load
├── dashboard/       One-shot HOME payload
├── insights/        Daily-win + proactive nudges
├── alerts/          AFib, critical HR, tachycardia rules
├── device/          Pair, firmware, last-seen heartbeat
├── push/            APNs token registration + schedule
├── alarms/          Smart-wake window + local fallback
├── reminders/       Water / Stand / Breathe / Bedtime / Move / WVI-drop
├── nps/             NPS submit + rescue reasons + referral tracking (new)
├── export/          Data-takeout (GDPR) → JSON/CSV bundle
├── family/          Shared pods + role-based reads
├── social/          Feed, challenges, leaderboards
├── health/          Liveness + readiness probes
├── audit/           Append-only audit log
└── reports/         Weekly / monthly PDF + plaintext summaries
```

---

## Quick start

### Prerequisites

- Rust 1.94+ (`rustup update stable`)
- Docker Desktop (for local Postgres / Redis)
- Optional: `claude` CLI in `$PATH` for AI endpoints (warm-cache helper)

### Local dev

```bash
git clone git@github.com:wellex-io/app-backend.git
cd app-backend
cp .env.example .env         # edit DATABASE_URL, SENTRY_DSN, PRIVY_APP_ID

# Bring up Postgres + Redis + Kafka
docker compose up -d db redis zookeeper kafka

# Run migrations (schema bootstraps automatically on first query too)
cargo run --release
# ► Server listening on http://0.0.0.0:8091
```

Smoke test:

```bash
curl -s http://localhost:8091/api/v1/health/live
# {"status":"ok"}

curl -s http://localhost:8091/api/v1/biometrics/hrv-detail \
  -H 'Authorization: Bearer <privy-jwt>'
# {"success":true,"data":{"rmssd":44.2,"sdnn":82.1,"lnrmssd":3.79,"pnn50":11.4}}
```

### Production deploy

The canonical production host is `api.wvi.wellex.io` (CherryServers aarch64). Deploy with:

```bash
# Sync source
rsync -az --exclude target/ ./ alex@<host>:/home/alex/wvi-api-rust/

# Rebuild & recreate container
ssh alex@<host> "cd /home/alex/wvi-api-rust && docker compose up -d --build api"
```

Traefik terminates TLS at the ingress and routes `:443` → container `:8091`.

---

## Endpoint reference (127 total)

Full schema is auto-served at **`/api/docs`** (OpenAPI 3.1 via `utoipa`).

Below is the high-level map by domain; grep `main.rs` for the exact registration order.

### Auth (4)
| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/auth/verify` | Verify Privy ID token + sync user row |
| GET | `/api/v1/auth/me` | Canonical identity lookup |
| POST | `/api/v1/auth/link-wallet` | Attach EVM wallet to Privy DID |
| POST | `/api/v1/auth/logout` | Server-side token invalidation |

### Users (5)
| Method | Path | Purpose |
|---|---|---|
| GET, PUT | `/api/v1/users/me` | Profile CRUD |
| GET | `/api/v1/users/me/norms` | Personalised norm baselines |
| POST | `/api/v1/users/me/norms/calibrate` | Rebuild norms from last 30 d |
| GET, PUT | `/api/v1/users/me/persona` | Ectomorph / endomorph / mesomorph archetype |

### Biometrics (23)
All biometric verticals share the `(AuthUser, State<PgPool>) → Json` handler shape.

Tables: `heart_rate`, `hrv`, `spo2`, `temperature`, `sleep_records`, `ppi`, `ecg`, `activities`, `blood_pressure`.

New in this release:

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/biometrics/hrv-detail` | rmssd / sdnn / lnrmssd / pnn50 (7 d avg) |
| GET | `/api/v1/biometrics/recovery-detail` | 7 d recovery trend + 24 h RR |
| GET | `/api/v1/biometrics/calories-detail` | active + bmr + total + goal |
| GET | `/api/v1/biometrics/vo2-detail` | last 4 monthly VO2 averages |
| GET | `/api/v1/biometrics/cardio-summary` | latest HR + HRV + BP in one call |
| GET | `/api/v1/biometrics/bio-age-detail` | 4-factor breakdown + aging_rate |

### WVI — the core scoring engine (11)
Proprietary 10-metric Wellness Vitality Index with personalised weighting.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/wvi/current` | Latest instantaneous score |
| GET | `/api/v1/wvi/history` | Raw series — `?granularity=daily` aggregates to one row/day |
| GET | `/api/v1/wvi/streak` | Consecutive-days-with-score counter |
| GET | `/api/v1/wvi/trends` | 7 / 30 / 90 day moving averages |
| GET | `/api/v1/wvi/predict` | Tomorrow's projected WVI given sleep/recovery trajectory |
| POST | `/api/v1/wvi/simulate` | What-if: "if I sleep 8 h tonight, WVI goes +N" |
| GET | `/api/v1/wvi/circadian` | Hourly WVI curve |
| GET | `/api/v1/wvi/correlations` | Pearson correlations between metrics |
| GET | `/api/v1/wvi/breakdown` | Decomposition: stress/sleep/recovery contribution |
| GET | `/api/v1/wvi/compare` | Today vs yesterday / last week / last month |
| POST | `/api/v1/wvi/backfill` | Recompute historical WVI from raw biometrics |

**Plus 5 WVI-v3 endpoints** (`/api/v1/wvi/v3/...`) for the personalised 18-component model.

### Emotions (10)
18-component fuzzy logic classifier — see `src/emotions/engine.rs`.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/emotions/current` | Live dominant emotion + confidence |
| GET | `/api/v1/emotions/history` | 7 d raw series |
| GET | `/api/v1/emotions/distribution` | Count by primary emotion |
| GET | `/api/v1/emotions/heatmap` | Weekly 7×24 grid |
| GET | `/api/v1/emotions/today-hourly` | Dominant emotion per hour today (new) |
| GET | `/api/v1/emotions/transitions` | Top 20 from→to flips |
| GET | `/api/v1/emotions/triggers` | Rank triggers with wvi_delta evidence |
| GET | `/api/v1/emotions/wellbeing` | 24 h positive-emotion ratio |
| GET | `/api/v1/emotions/streaks` | Consecutive positive-day streaks |
| GET | `/api/v1/emotions/v2/intraday` | Micro-pulse detection |

### AI (8)
Each AI endpoint hits a pre-warmed `claude` CLI process; first response is served from the 5-minute prewarmer cache, subsequent ones stream live.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/ai/morning-brief` | Sonnet — next-best-action recommendation |
| GET | `/api/v1/ai/evening-review` | Sonnet — what happened today + tomorrow primer |
| GET | `/api/v1/ai/body-story` | Opus — long-form narrative from HR/HRV/SpO2 trends |
| GET | `/api/v1/ai/recovery-deep` | Opus — clinician-grade recovery assessment |
| GET | `/api/v1/ai/ecg-interpret` | Opus — ECG rhythm interpretation |
| GET | `/api/v1/ai/weekly-deep` | Opus — week-over-week deltas + causality |
| GET | `/api/v1/ai/full-analysis` | Opus — full-body holistic report |
| POST | `/api/v1/ai/ask` | Free-form chat with full context injection |

### NPS / Rescue / Referral (3, new)
| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/nps/submit` | { score 0-10, touchpoint } |
| POST | `/api/v1/rescue/submit` | { reason, score } for detractor rescue flow |
| POST | `/api/v1/referrals/track` | { code, channel } for promoter share tracking |

Tables self-bootstrap on first write via `CREATE TABLE IF NOT EXISTS`.

### …plus 60+ endpoints across
**Activities · Sleep · Intraday · Training-load · Risk · Dashboard · Insights · Alerts · Stress-v2 · Device · Push · Alarms · Reminders · Export · Family · Social · Health · Settings · Audit · Events · Reports · Sensitivity**

Full list in `src/main.rs` (`.route(...)` registrations, ~300 lines).

---

## Data model highlights

- **`users`** — canonical identity (`privy_did` → `uuid id`). Referenced by every biometric row via FK.
- **`hrv`** — the hottest table. Stores RR-derived: rmssd, sdnn, pnn50, stress, recovery_score, breathing_rate, systolic_bp, diastolic_bp, bp_source, vo2_max. Indexed on `(user_id, timestamp DESC)`.
- **`heart_rate`** — minute-resolution bpm samples. GiST index on timestamp for range scans.
- **`wvi_scores`** — materialised WVI per minute. Backfill scheduler recomputes incrementally.
- **`sleep_records`** — per-night aggregation (total_hours, deep, rem, light, efficiency). Date-keyed.
- **`emotions`** — primary + secondary classification + full 18-component scores blob.
- **`activities`** — workout sessions (start_time, duration, calories, distance, activity_type).

Schema lives inline in query strings (one-file-per-module style) and is applied with `CREATE TABLE IF NOT EXISTS` idempotency — no migration framework, just compiled SQL that runs on boot.

---

## Running the background schedulers

Several async workers run alongside the HTTP server inside the same binary:

| Worker | Purpose | Cadence |
|---|---|---|
| `wvi::scheduler::backfill` | Recompute WVI from raw biometrics | every 3 h |
| `ai::cli::prewarmer` | Warm Claude prompt cache | every 5 min |
| `alerts::rules` | Evaluate AFib / HR crisis rules | every minute per active user |
| `sleep::roll_up` | Close yesterday's sleep record at 11:00 | once per day |

All schedulers live in their module's `mod.rs` and are spawned from `main.rs` via `tokio::spawn`.

---

## Observability

```bash
# Grafana (Prometheus + Loki panels)
https://grafana.wellex.internal

# Sentry errors
https://sentry.io/wellex-io/wvi-api

# Live docker logs
ssh alex@<host> "docker logs -f wvi-api"
```

Every handler is instrumented with `tracing::instrument(...)` so each request carries `request_id`, `privy_did`, `latency_ms`, and `status` into the structured log.

---

## Testing

```bash
# Unit tests (pure functions — BP tier classifier, WVI math, emotion fuzzy logic)
cargo test --lib

# Integration tests against a live test database
WVI_TEST_DATABASE_URL=postgres://... cargo test --test '*' -- --test-threads=1
```

BP tier classification, VO2 age-matching, WVI v3 component weights, and emotion fuzzy edges are covered in `#[cfg(test)] mod` blocks alongside their production code.

---

## Contributing

This repo is the source of truth for the Wellex API. Branching model:

- `main` — production (auto-deployed via `docker compose up -d --build api` on the server)
- feature branches → PR → squash merge

Commit message style: `feat(<module>): <short>` — see recent commits for examples.

---

## License

Proprietary — © Wellex.io 2026. All rights reserved.
