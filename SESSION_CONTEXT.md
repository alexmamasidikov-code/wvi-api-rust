# Wellex dev session — canonical context (2026-04-22)

**Purpose:** single-file handoff. Read this after `/compact` to recover everything you need to continue working.

---

## Where everything lives

### Repos (canonical = wellex-io org)
| What | Canonical | Local clone | Mirror |
|---|---|---|---|
| Rust API | `wellex-io/app-backend` (branch `main`) | `/Users/alexander/Code/wvi-api-rust` | `alexmamasidikov-code/wvi-api-rust` → remote `archive` |
| iOS app (monorepo layout) | `wellex-io/app-frontend` (branch `main`, code under `iOS/`) | `/Users/alexander/Code/wellex-app-frontend` (monorepo clone) | — |
| iOS primary dev repo (flat Xcode layout) | `alexmamasidikov-code/wvi-health-ios` | `/Users/alexander/Code/WVIHealth` | mirrored to wellex-io via script |

**Backend push:**
```bash
cd /Users/alexander/Code/wvi-api-rust
git push origin master:main          # → wellex-io/app-backend (default)
git push archive master               # → alexmamasidikov-code (mirror)
```

**iOS push:**
```bash
cd /Users/alexander/Code/WVIHealth
git push origin main                                    # → alexmamasidikov-code
./scripts/mirror-to-wellex.sh "chore: <message>"        # → wellex-io/app-frontend iOS/ subfolder
```

### Server
- **Host:** `apidev.wellex.io` (resolves to Cloudflare IPs `104.21.7.48`, `172.67.135.192` → origin `88.216.62.20`)
- **Origin:** Ampere-1a ARM64, 12 cores, 54 GiB RAM, 290 GiB disk (69 used), Ubuntu 24.04, no GPU
- **SSH:** `ssh wellex_dev` (alias → user `wellex` @ `88.216.62.20`, key `~/.ssh/alex_remote`)
- **GitHub deploy key (for future server-side clones):** `~/.ssh/wellex_github_deploy` on the server, not yet added to wellex-io repos — currently the server gets code via `rsync` from local
- **TLS:** wildcard `*.wellex.io` cert in Traefik (`/srv/wellex/traefik/certs`)

### External URLs
- `wviapi.wellex.io` → Traefik → wvi-api container (port 8091)
- Old: `api.wvi.internal` on `100.90.71.111` — **deactivated**

---

## Docker stack on apidev (`/srv/wellex/`)

```
traefik     v3.3     ingress, 80/443                 healthy
postgres    TSDB16   shared by all wellex dev        healthy  (DB `wvi` owned by role `wvi`)
redis       7-alpine shared                          healthy
wvi-api     wvi-api:dev (ours)                       healthy   /srv/wellex/apps/wvi-api/
loki        latest   log aggregation                 up
grafana     11.1     metrics + log dashboards        up
cadvisor    latest   container metrics               up
node-exp    latest   node metrics                    up
qdrant      v1.13.2  vector DB (Wellex site)         up
appdev      nginx    placeholder for appdev.wellex.io up
apidev      other Rust Wellex MLM API (not ours!)    up    /api/v1/health + /metrics
```

**wvi-api compose:** `/srv/wellex/apps/wvi-api/docker-compose.wvi.yml` (mirrored to repo as `docker-compose.wellex-dev.yml`).

Env: `/srv/wellex/apps/wvi-api/.env.host` (copy of `/srv/wellex/.env`).

### Key env overrides in wvi-api compose
```yaml
DATABASE_URL:            postgres://wvi:wvi_dev_2026@postgres:5432/wvi
REDIS_URL:               redis://:${REDIS_PASSWORD}@redis:6379/1
API_BIND:                0.0.0.0:8091
SERVER_URL:              https://wviapi.wellex.io
RATE_LIMIT_PER_SEC_READ:  1000
RATE_LIMIT_PER_SEC_WRITE: 500
KAFKA_ENABLED:            false   # Kafka not migrated
KAFKA_BROKERS:            ""
SENTRY_DSN:               (empty — Sentry not wired yet in this env)
```

### Traefik routes (`/srv/wellex/traefik/dynamic/routers.yml`)
`wviapi.wellex.io` → service `wviapi-svc` → `http://wvi-api:8091`
- middlewares: `security-headers` only (no rate-limit-api — rate limiting handled in Rust)

---

## Data in wvi DB (as of 2026-04-22 18:24 UTC)
| Table | Rows |
|---|---|
| heart_rate | 510 796 |
| temperature | 391 697 |
| hrv | 59 193 |
| wvi_scores | 38 964 |
| emotions | 52 092 |
| audit_log | 52 705 |
| stress_samples_1min | 7 955 |
| emotion_samples_1min | 7 928 |
| spo2 | 5 068 |
| activity | 3 444 |

Migrated from `100.90.71.111:wvi-db` → pg_dump → restored here; `ALTER TABLE … OWNER TO wvi` applied to all public tables (sqlx migrations were failing on `permission denied for _sqlx_migrations` before that).

---

## Recent fixes this session (keep in mind)

1. **Migration + deploy** (branchless docker, reused shared postgres/redis on wellex-net).
2. **Traefik rate-limit-api removed** from `wviapi` router (was 100/min — tripped iOS bulk sync).
3. **Rust rate limit** raised to READ=1000/s, WRITE=500/s per bearer (was 20 writes/s default → triggered 429 cascade from bulk chunks).
4. **cardio-summary SQL cast `bpm::int4`** (`src/biometrics/handlers.rs`): TimescaleDB stores `REAL`, handler wanted `INT4` → 500. Only that one handler had the mismatch; all others already decode to f32/f64.
5. **iOS `bulkUploadRecords` retry state machine** (`WVIHealth/Features/Device/WellexDeviceManager.swift` ~line 2421):
   - `maxTransientAttempts = 3`, `maxTokenRefreshes = 1`, `baseBackoffMs = 500`
   - 401 → refresh token + retry once (doesn't consume transient budget)
   - 429 / 5xx / transport error → exponential backoff 500 / 1000 / 2000 ms
   - Body serialized **once** per chunk (not per attempt)
   - Scoped banner clear: `syncHistoryLastError = nil` only when current banner's prefix matches label (HR success can't wipe HRV error)
6. **Kafka disabled** in compose (was emitting 1 ERROR every 30s in logs).
7. **wellex-io = canonical origin** on local Rust clone (old alexmamasidikov remote → `archive`).

---

## Verified clean state at snapshot time

```
wvi-api       running, 0 restarts, 57 MiB RAM, 1.25% CPU, 0 ERRORs in 5 min
Traefik last hour:  5072× 200, 6× 502 (all in my deploy windows), 2× 500 (cardio bug, now fixed)
iOS → wviapi.wellex.io verified (AppConfig.productionURL)
20/21 iOS-consumed endpoints return 200 with real migrated data
     (only /dashboard/home 404 — legacy, not used by current iOS)
```

---

## Known next-up items (not urgent)

- Local AI model plan — `/Users/alexander/Code/wvi-api-rust/docs/AI_MODEL_PLAN.md`. Phase 1 = Moonshot Kimi K2 API integration (replace `claude` CLI shell-out in `src/ai/cli.rs`). Server has no GPU → cannot run Kimi/MiniMax locally in full size.
- 3 unit-test files (`OnboardingCoordinatorTests.swift`, `OrderStoreTests.swift`, `NPSStoreTests.swift`) already in WVIHealthTests + wired through `project.yml` + passing (29/29 ✓).
- ATT prompt implemented (`Core/Privacy/ConsentManager.swift → AppTrackingManager.requestIfNeeded()` called from `afterSignIn`).
- Sentry SDK 8.58 → 9.10 is a major bump (breaking breadcrumb API) — deferred.
- Privy SDK 2.11 already latest (no bump needed).
- `100.90.71.111` can be fully decommissioned after a few days of apidev stability (Postgres + Redis still running as backup snapshot).

---

## Build / test quick reference

**iOS build + install on TeryMarius:**
```bash
cd /Users/alexander/Code/WVIHealth
xcodebuild -project WVIHealth.xcodeproj -scheme WVIHealth \
           -destination "id=00008140-001659901123001C" \
           -derivedDataPath build/DerivedData build
xcrun devicectl device install app --device 954593F7-C779-5749-AD45-0C8AAFF58179 \
           "build/DerivedData/Build/Products/Debug-iphoneos/WVI Health.app"
```

**iOS unit tests (simulator):**
```bash
xcodebuild -project WVIHealth.xcodeproj -scheme WVIHealthTests \
           -destination "platform=iOS Simulator,name=iPhone 17 Pro" test
```

**iOS UI onboarding walkthrough test (device, with screenshots):**
```bash
xcodebuild -project WVIHealth.xcodeproj -scheme WVIHealthUITests \
           -destination "id=00008140-001659901123001C" \
           -only-testing:WVIHealthUITests/OnboardingWalkthroughUITests test
```

**Rust redeploy (full cycle from local):**
```bash
cd /Users/alexander/Code/wvi-api-rust
# 1. edit, commit, push
git push origin master:main
# 2. mirror to server
rsync -az --exclude='target/' --exclude='.git/' ./ wellex_dev:/srv/wellex/apps/wvi-api/
# 3. rebuild + recreate container
ssh wellex_dev "cd /srv/wellex/apps/wvi-api && docker compose -f docker-compose.wvi.yml --env-file .env.host up -d --build"
# 4. smoke test
curl -sk https://wviapi.wellex.io/api/v1/health/live
# 5. watch logs
ssh wellex_dev "docker logs -f wvi-api"
```

**TeryMarius device id:** `00008140-001659901123001C` (xcodebuild) / `954593F7-C779-5749-AD45-0C8AAFF58179` (devicectl).

---

## Credentials (references only — never paste here)

- `/srv/wellex/.env` on the server — all prod secrets (mode 600, read don't regenerate)
- `~/.ssh/alex_remote` — SSH key for wellex_dev
- `~/.ssh/wellex_github` — GitHub deploy key (read+write on both wellex-io repos)
- `~/.ssh/wvi_deploy` — SSH key for the old 100.90.71.111 Spark server (still works)
- Privy app id in `Core/Config.swift` (public, not a secret)

---

## Session commits summary

| Commit | Repo | What |
|---|---|---|
| `894bf33` | app-backend | cardio-summary + bio-age-detail endpoints |
| `c153b5b` | app-backend | hrv/recovery/calories/vo2 detail + today-hourly endpoints |
| `ef81813` | app-backend | NPS/rescue/referral endpoints |
| `5a35c69` | app-backend | docs/AI_MODEL_PLAN.md |
| `456c9f5` | app-backend | docker-compose.wellex-dev.yml overlay |
| `e0ceee1` | app-backend | cardio-summary bpm::int4 cast fix |
| `a3294af` | app-backend | disable Kafka client |
| `96bc08e` | app-frontend (iOS) | 13-item audit batch (locale, P0/P1 fixes) |
| `077b204` | app-frontend (iOS) | serverURL → wviapi.wellex.io |
| `e720464` | app-frontend (iOS) | same change mirrored to monorepo |
| `f685bff` | app-frontend (iOS) | bulkSync retry 5xx + 429 (initial) |
| `3b8c408` | app-frontend (iOS) | reviewer-fix retry state machine |
| `0df89d5` | WVIHealth | scripts/mirror-to-wellex.sh |
| `6fd8294` | app-frontend | mirrored the script |
