# WVI API — Rust Backend Architecture

> **English version:** see [ARCHITECTURE.en.md](ARCHITECTURE.en.md)

Rust API для биометрического health-приложения Wellex. Axum + PostgreSQL + Redis + Kafka. Считает WVI v2 из 12 биометрических метрик, детектит 18 эмоций через fuzzy logic, рассчитывает computed-индикаторы (Bio Age, VO2 Max, Coherence, Training Load, BP, Sleep Score).

---

## Содержание

1. [Обзор](#обзор)
2. [18 модулей](#18-модулей)
3. [WVI v2 алгоритм](#wvi-v2-алгоритм)
4. [Emotion Engine](#emotion-engine)
5. [Computed metrics](#computed-metrics)
6. [API Reference (108 routes)](#api-reference-108-routes)
7. [Infrastructure](#infrastructure)
8. [Observability](#observability)
9. [Security hardening](#security-hardening)
10. [Setup / Run / Deploy](#setup--run--deploy)

---

## Обзор

**Stack:** Rust 1.88+ · Axum · Tokio · PostgreSQL 16 · Redis 7 · Kafka · Prometheus · Kong · Kubernetes · ArgoCD

**Размер:** 53 `.rs` файла · ~5,254 строки · **108 routes + metrics endpoint**

**Порт:** 8091 (host network в Docker)

### Архитектурная диаграмма

```mermaid
graph TB
    Client[iOS App]
    Kong[Kong API Gateway]
    API[Rust API :8091]
    DB[(PostgreSQL 5433)]
    Redis[(Redis cache)]
    Kafka[Kafka event bus]
    Prom[Prometheus :9090]
    Graf[Grafana :3000]
    OR[OpenRouter Gemini Flash]

    Client -->|HTTPS zrok| Kong
    Kong -->|rate limit, CORS| API
    API -->|SQLx| DB
    API -->|cache 5min| Redis
    API -->|events| Kafka
    API -->|AI requests| OR
    API -->|/metrics| Prom
    Prom --> Graf
    Kafka -->|consumers| API
```

---

## 18 модулей

```
src/
├── main.rs              # Router, middleware, Kafka, rate limiter wiring
├── config.rs            # Environment config + DB pool
├── error.rs             # APIError + IntoResponse
├── cache.rs             # In-memory cache (user_id 5min, AI 10min)
├── audit.rs             # Audit log table + GET /audit/log
├── events.rs            # Kafka EventBus (5 topics)
├── metrics.rs           # Prometheus /metrics endpoint
│
├── auth/                # Privy OAuth verification
├── users/               # User profile + norms (personal biometric baselines)
├── biometrics/          # HR, HRV, SpO2, Temp, PPI, ECG, Sleep, Activity ingest + get
│   ├── computed.rs      # BP, VO2 Max, Bio Age, Coherence, Training Load, Sleep Score
│   └── handlers.rs      # 25 endpoints
├── wvi/                 # WVI v2 calculator
│   ├── calculator.rs    # Geometric mean + progressive sigmoid + 12 metrics + hard caps
│   ├── normalizer.rs    # Individual metric → 0-100 score
│   ├── models.rs        # WVILevel enum (7 tiers)
│   └── handlers.rs      # 9 endpoints (current, history, trends, breakdown, etc.)
├── emotions/            # 18 emotions fuzzy logic
│   ├── engine.rs        # Sigmoid + bell curves — source of truth
│   ├── models.rs        # EmotionState enum (18), EmotionResult
│   └── handlers.rs      # 8 endpoints (current, history, wellbeing, distribution, etc.)
├── activities/          # Steps, calories, distance, zones, workouts
├── sleep/               # Sleep analysis (7 endpoints)
├── ai/                  # 7 AI endpoints via OpenRouter Gemini Flash
├── reports/             # Weekly aggregates + PDF generation
├── alerts/              # Active + history + settings
├── device/              # Bracelet registration + firmware + auto-monitoring
├── training/            # Training recommendations from HRV/HR
├── risk/                # 5 risk assessment endpoints
├── export/              # CSV / JSON / Health Summary
├── social/              # Feed, post, challenges, leaderboard
├── health/              # /health, /ready, /live
├── settings/            # User settings + notifications
└── dashboard/           # Widgets, daily brief, evening review
```

---

## WVI v2 алгоритм

**Формула:**
```
wvi_score = emotion_multiplier × progressive_curve(geometric_mean_weighted(12 metrics))
          → clamp [0, 100]
```

### 12 взвешенных метрик (сумма весов = 1.00)

| Метрика | Вес | Источник | Range |
|---|---|---|---|
| HRV RMSSD | 16% | `hrv_rmssd` (ms) | 0-150 |
| Stress index | 14% | 100 - min(100, hrv/0.7) | 0-100 |
| Sleep score | 13% | `sleep_score` | 0-100 |
| Emotion score | 12% | `emotion_score` | 0-100 |
| SpO2 | 8% | `spo2` (%) | 85-100 |
| Heart rate delta | 7% | `|HR - resting_HR|` | low better |
| Steps | 7% | `steps` time-proportional | 0-15k/day |
| Active calories | 6% | `active_calories` | 0-1000/day |
| ACWR (acute:chronic) | 5% | Training load ratio | 0.8-1.5 |
| Blood pressure | 5% | Systolic / diastolic | 120/80 ideal |
| Temperature delta | 4% | `temp - base_temp` | ±0.5°C |
| PPI coherence | 3% | `ppi_coherence` | 0.0-1.0 |

### Weighted geometric mean
```rust
fn geometric_mean(scores: &[(String, f64, f64)]) -> f64 {
    let sum_w: f64 = scores.iter().map(|(_, _, w)| w).sum();
    let ln_sum: f64 = scores.iter()
        .map(|(_, s, w)| w * s.max(1.0).ln())
        .sum();
    (ln_sum / sum_w).exp()
}
```

Геометрическое среднее штрафует низкие значения сильнее арифметического — плохой HRV не компенсируется хорошим SpO2.

### Progressive sigmoid curve (≥60)
```rust
fn progressive_curve(x: f64) -> f64 {
    if x <= 60.0 { x }
    else {
        60.0 + 40.0 * (1.0 - f64::exp(-3.5 * (x - 60.0) / 40.0))
    }
}
```

Делает прогресс выше 60 заметным (60 → 75, 80 → 93, 100 → 98.8) — мотивация держать excellent.

### Emotion multipliers (18 эмоций)

| Positive (>1.0) | Multiplier | Negative (<1.0) | Multiplier |
|---|---|---|---|
| Flow | 1.15 | Drowsy | 0.95 |
| Meditative | 1.10 | Sad | 0.90 |
| Joyful | 1.08 | Frustrated | 0.90 |
| Excited | 1.05 | Stressed | 0.90 |
| Energized | 1.05 | Anxious | 0.85 |
| Relaxed | 1.04 | Angry | 0.82 |
| Focused | 1.03 | Fearful | 0.80 |
| Calm | 1.02 | Exhausted | 0.78 |
| Recovering | 1.00 (neutral) | Pain | 0.78 |

### Hard caps (override progressive)
- SpO2 < 92% → ceiling 70 (никакой другой метрикой не перебить)
- HR delta > 30 bpm → ceiling 75
- Stress > 80 → ceiling 65
- Temperature delta > 1.5°C → ceiling 70

### Формула version
`formula_version: "2.0"` — увеличивается при изменении весов или curve.

---

## Emotion Engine

**Источник:** `src/emotions/engine.rs`

18 эмоций, каждая — функция 13 биометрических параметров:
```rust
pub fn detect(
    heart_rate: f64, resting_hr: f64, hrv: f64, stress: f64,
    spo2: f64, temperature: f64, base_temp: f64,
    systolic_bp: f64, ppi_coherence: f64, ppi_rmssd: f64,
    sleep_score: f64, activity_score: f64, hrv_trend: f64,
    prev_emotion: Option<EmotionState>, elapsed_secs: f64,
) -> EmotionResult
```

### Fuzzy math primitives

```rust
// Сигмоида — плавный переход 0→1 вокруг mid
fn sigmoid(x: f64, mid: f64, k: f64) -> f64 {
    1.0 / (1.0 + (-k * (x - mid)).exp())
}

// Обратная сигмоида
fn sigmoid_inv(x: f64, mid: f64, k: f64) -> f64 {
    1.0 / (1.0 + (k * (x - mid)).exp())
}

// Колокол — max в center, спад в обе стороны
fn bell(x: f64, center: f64, width: f64) -> f64 {
    (-(x - center).powi(2) / (2.0 * width.powi(2))).exp()
}
```

### Пример правила — Angry

```rust
let s = sigmoid(stress, 65.0, 0.15)           // stress > 65 → boost
      * sigmoid(delta_hr, 22.0, 0.12)         // pulse way above resting
      * sigmoid_inv(hrv, 38.0, 0.10)          // HRV suppressed
      * sigmoid(systolic_bp, 130.0, 0.08)     // elevated BP
      * sigmoid_inv(ppi_coherence, 0.35, 8.0) // low coherence
      * sigmoid(temp_delta, 0.2, 5.0);        // slight warming
```

Результат — `EmotionCandidate { emotion: Angry, score: s, weight: 1.0 }`. Побеждает тот с наибольшим `score × weight`.

### Temporal damping

Если `prev_emotion == current` → weight ×1.1. Если `elapsed_secs < 30` → текущая эмоция держится ×1.05. Предотвращает "мельтешение" эмоций.

### Все 18 эмоций

Angry · Anxious · Stressed · Frustrated · Fearful · Sad · Exhausted · Pain · Drowsy · Recovering · Calm · Relaxed · Focused · Meditative · Joyful · Excited · Energized · Flow

---

## Computed metrics

**Источник:** `src/biometrics/computed.rs`

6 derived показателей, обновляются после каждого biometrics sync.

| Индикатор | Формула (упрощённо) | Range |
|---|---|---|
| **Blood Pressure** | Estimate из PPI + возраст | 90/60 — 180/110 |
| **VO2 Max** | Tanaka + resting HR correction | 20-60 ml/kg/min |
| **Bio Age** | Chronological age ± HRV/HR/sleep deviation (требует ≥7 дней) | ±10 лет |
| **Coherence** | PPI power spectrum (0.04-0.26 Hz peak) | 0.0-1.0 |
| **Training Load** | TRIMP × duration × HR zones | capped 120min/200 TRIMP |
| **Sleep Score** | Phase distribution + duration + efficiency | 0-100 |

---

## API Reference (108 routes)

Все endpoints под `/api/v1/`. Требуют `Authorization: Bearer <token>` (кроме health/docs).

### Auth (4)
| Method | Path | Handler |
|---|---|---|
| POST | `/auth/verify` | Privy token verification |
| GET | `/auth/me` | Current user |
| POST | `/auth/link-wallet` | Link crypto wallet |
| POST | `/auth/logout` | Invalidate session |

### Users (3)
| Method | Path |
|---|---|
| GET / PUT | `/users/me` |
| GET | `/users/me/norms` |
| POST | `/users/me/norms/calibrate` |

### Biometrics (25)
- `POST /biometrics/sync` — bulk ingest
- `GET/POST /biometrics/{heart-rate,hrv,spo2,temperature,sleep,ppi,ecg,activity}` (8×2 = 16)
- `GET /biometrics/{blood-pressure,stress,breathing-rate,rmssd,coherence,computed,recovery,realtime,summary}` (9)

### WVI (9)
| Method | Path | Purpose |
|---|---|---|
| GET | `/wvi/current` | Live WVI score + breakdown |
| GET | `/wvi/history` | Historical scores |
| GET | `/wvi/trends` | 7/30-day trend |
| GET | `/wvi/predict` | ML-based 24h forecast |
| POST | `/wvi/simulate` | What-if scenarios |
| GET | `/wvi/circadian` | Daily rhythm pattern |
| GET | `/wvi/correlations` | Metric correlation matrix |
| GET | `/wvi/breakdown` | Per-metric score |
| GET | `/wvi/compare` | Peer comparison |

### Emotions (8)
`current · history · wellbeing · distribution · heatmap · transitions · triggers · streaks`

### Activities (10)
`current · history · load · zones · categories · transitions · sedentary · exercise-log · recovery-status · manual-log`

### Sleep (7)
`last-night · score-history · architecture · consistency · debt · phases · optimal-window`

### AI (7) — via OpenRouter Gemini Flash
`interpret · recommendations · chat · explain-metric · action-plan · insights · genius-layer`

### Reports (5)
`generate · list · templates · {id} · {id}/download`

### Alerts (6)
`list · active · settings · history · {id}/acknowledge · stats`

### Device (6)
`status · auto-monitoring · sync · capabilities · measure · firmware`

### Training (4)
`recommendation · weekly-plan · overtraining-risk · optimal-time`

### Risk (5)
`assessment · anomalies · chronic-flags · correlations · volatility`

### Dashboard (3)
`widgets · daily-brief · evening-review`

### Export (3)
`csv · json · health-summary`

### Settings (2)
`/settings · /settings/notifications`

### Audit (1)
`/audit/log`

### Social (4)
`feed · post · challenges · leaderboard`

### Health (4)
`server-status · api-version · ready · live`

### Docs (1) + Metrics (1)
`/api/v1/docs.json` (OpenAPI spec) · `/metrics` (Prometheus)

**Полный router:** см. `src/main.rs`.

---

## Infrastructure

### `docker-compose.yml` — 8 services

| Service | Image | Port | Purpose |
|---|---|---|---|
| **api** | Built from Dockerfile | 8091 (host network) | Rust API |
| **db** | postgres:16 | 5433 | Main DB |
| **redis** | redis:7 | 6379 | Cache |
| **kong** | kong/kong-gateway:latest | 8000/8001 | API Gateway (rate limiting, CORS, security headers, bot detection) |
| **prometheus** | prom/prometheus | 9090 | Metrics scraping |
| **grafana** | grafana/grafana | 3000 | Dashboards |
| **zookeeper** | confluentinc/cp-zookeeper | 2181 | Kafka coordinator |
| **kafka** | confluentinc/cp-kafka | 9092 | Event bus (5 topics: biometrics, emotions, wvi, alerts, audit) |

### Kubernetes (`kubernetes/`)
- `deployment.yaml` — replicas: 3, resource limits
- `hpa.yaml` — HPA 3→100 based on CPU (70%) / memory (80%)
- `service.yaml` — ClusterIP
- `ingress.yaml` — nginx ingress (TLS termination pending `api.wellex.ai`)
- `secrets.yaml` — DB password, OpenRouter key, Privy secrets

### Redis Cluster (`redis-cluster/`)
- `docker-compose.redis-cluster.yml` — 6-node cluster (3 masters + 3 replicas)
- Sharded by user_id hash
- Sub-millisecond reads under load

### ArgoCD GitOps (`argocd/`)
- `application.yaml` — auto-sync from `master` branch
- `install.sh` — one-shot Helm install

### Kong declarative config (`kong/kong.yml`)
- Rate limiting: 60 req/min (authed), 20 req/min (anon)
- CORS: whitelist `zrok.io`, `localhost:3000`
- Security headers: X-Content-Type-Options, X-Frame-Options, X-XSS-Protection, Referrer-Policy
- Body size limit: 5MB
- Bot detection plugin

---

## Observability

### Prometheus metrics (`/metrics` endpoint)
- `wvi_api_requests_total{method,path,status}` — counter
- `wvi_api_request_duration_seconds{path}` — histogram
- `wvi_api_wvi_calculations_total` — counter
- `wvi_api_emotion_detections_total{primary}` — counter
- `wvi_api_active_users` — gauge

### Grafana dashboards
- Request rate + p50/p95/p99 latency
- Error rate by endpoint
- WVI distribution (histogram of scores)
- Emotion distribution (top emotions today)
- DB pool usage

### Kafka event bus (5 topics)
- `biometrics.ingested` — каждый `POST /biometrics/sync`
- `emotions.detected` — после emotion engine run
- `wvi.calculated` — после WVI v2 compute
- `alerts.fired` — threshold breaches
- `audit.logged` — auth/config changes

### Audit log (`audit` table + `GET /api/v1/audit/log`)
Fields: user_id, action, resource, timestamp, ip_address, user_agent.

### Structured JSON logging (tracing crate)
Default level `info`. Fields: timestamp, level, target, span, message.

---

## Security hardening

- **Per-user rate limiting** — tower-governor 60 req/min authed, 20 anon
- **Request body limit** — 5MB (Kong level + app level)
- **CORS** — whitelist origins only
- **Security headers** — X-Content-Type-Options, X-Frame-Options, X-XSS-Protection, Referrer-Policy, Strict-Transport-Security
- **Audit logging** — all auth/settings/sync actions
- **Graceful shutdown** — SIGTERM waits for in-flight requests, closes DB pool, drains Kafka
- **Health endpoints** — `/health`, `/ready`, `/live` для K8s liveness/readiness probes
- **Secrets** — через `kubernetes/secrets.yaml` + env vars, never in code
- **SSL pinning** — на клиенте (iOS) против zrok.io domain

---

## Setup / Run / Deploy

### Требования
- Rust 1.88+ (`rustup install 1.88`)
- Docker 24+ с compose plugin
- PostgreSQL client (`psql`) для миграций
- `sqlx-cli` для migrations: `cargo install sqlx-cli`

### Локальная разработка

```bash
git clone https://github.com/alexmamasidikov-code/wvi-api-rust.git
cd wvi-api-rust

# Запустить всю инфру
docker compose up -d db redis

# Миграции
DATABASE_URL=postgres://wellex:password@localhost:5433/wvi-db \
  sqlx migrate run

# API локально
cargo run
```

### Тесты

```bash
# Все тесты
cargo test --bin wvi-api

# Конкретный модуль
cargo test --bin wvi-api wvi::calculator::tests

# С логами
RUST_LOG=debug cargo test --bin wvi-api -- --nocapture
```

### Docker build + полный stack

```bash
# Локально полный стек (8 сервисов)
docker compose up -d

# Логи API
docker compose logs -f api

# Остановить
docker compose down
```

### Production deploy (100.90.71.111)

```bash
# SSH на сервер
ssh root@100.90.71.111

# Обновить код
cd /opt/wvi-api-rust
git pull origin master

# Пересобрать и перезапустить
docker compose up -d --build api

# Проверить
curl https://6ssssdj5s38h.share.zrok.io/api/v1/health/server-status
```

### Kubernetes

```bash
# Создать namespace + secrets
kubectl create namespace wvi
kubectl apply -f kubernetes/secrets.yaml

# Deploy
kubectl apply -f kubernetes/deployment.yaml
kubectl apply -f kubernetes/service.yaml
kubectl apply -f kubernetes/ingress.yaml
kubectl apply -f kubernetes/hpa.yaml

# Статус
kubectl -n wvi get pods
kubectl -n wvi logs -l app=wvi-api -f
```

### ArgoCD (GitOps)

```bash
# Installer
./argocd/install.sh

# Применить манифест
kubectl apply -f argocd/application.yaml

# Watch sync
kubectl -n argocd get applications
```

### Load testing

```bash
# k6 установка
brew install k6

# Запуск
k6 run loadtest.k6.js

# Результаты — p95, error rate, RPS
```

Текущий baseline: **p95 = 208ms** при 1000 RPS.

---

## Связанные репозитории

- **iOS client:** [alexmamasidikov-code/wvi-health-ios](https://github.com/alexmamasidikov-code/wvi-health-ios)
- **Этот репо (API):** [alexmamasidikov-code/wvi-api-rust](https://github.com/alexmamasidikov-code/wvi-api-rust)

---

**Последнее обновление:** 2026-04-16 · **Commit:** `34e5418` (15 unit tests for WVI + Emotion)
