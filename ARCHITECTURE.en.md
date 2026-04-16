# WVI API — Rust Backend Architecture (English)

> **Русская версия:** см. [ARCHITECTURE.md](ARCHITECTURE.md)

Rust API for Wellex biometric health app. Axum + PostgreSQL + Redis + Kafka. Computes WVI v2 from 12 biometric metrics, detects 18 emotions via fuzzy logic, derives computed indicators (Bio Age, VO2 Max, Coherence, Training Load, BP, Sleep Score).

---

## Table of Contents

1. [Overview](#overview)
2. [18 modules](#18-modules)
3. [WVI v2 algorithm](#wvi-v2-algorithm)
4. [Emotion Engine](#emotion-engine)
5. [Computed metrics](#computed-metrics)
6. [API Reference (108 routes)](#api-reference-108-routes)
7. [Infrastructure](#infrastructure)
8. [Observability](#observability)
9. [Security hardening](#security-hardening)
10. [Setup / Run / Deploy](#setup--run--deploy)

---

## Overview

**Stack:** Rust 1.88+ · Axum · Tokio · PostgreSQL 16 · Redis 7 · Kafka · Prometheus · Kong · Kubernetes · ArgoCD

**Size:** 53 `.rs` files · ~5,254 lines · **108 routes + metrics endpoint**

**Port:** 8091 (host network inside Docker)

### Architecture diagram

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

## 18 modules

See the RU version for the full annotated tree. Same structure.

---

## WVI v2 algorithm

**Formula:**
```
wvi_score = emotion_multiplier × progressive_curve(geometric_mean_weighted(12 metrics))
          → clamp [0, 100]
```

### 12 weighted metrics (weights sum to 1.00)

| Metric | Weight | Source | Range |
|---|---|---|---|
| HRV RMSSD | 16% | `hrv_rmssd` (ms) | 0-150 |
| Stress index | 14% | 100 - min(100, hrv/0.7) | 0-100 |
| Sleep score | 13% | `sleep_score` | 0-100 |
| Emotion score | 12% | `emotion_score` | 0-100 |
| SpO2 | 8% | `spo2` (%) | 85-100 |
| Heart rate delta | 7% | `|HR - resting_HR|` | lower is better |
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

Geometric mean penalizes low values more than arithmetic — a poor HRV can't be compensated by a great SpO2.

### Progressive sigmoid curve (≥60)
```rust
fn progressive_curve(x: f64) -> f64 {
    if x <= 60.0 { x }
    else {
        60.0 + 40.0 * (1.0 - f64::exp(-3.5 * (x - 60.0) / 40.0))
    }
}
```

Makes progress above 60 visible (60 → 75, 80 → 93, 100 → 98.8) — motivates sustaining excellent performance.

### Emotion multipliers (18 emotions)

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

### Hard caps (override progressive curve)
- SpO2 < 92% → ceiling 70
- HR delta > 30 bpm → ceiling 75
- Stress > 80 → ceiling 65
- Temperature delta > 1.5°C → ceiling 70

### Formula version
`formula_version: "2.0"` — bumped on weight or curve changes.

---

## Emotion Engine

See RU version — same sigmoid/bell math, same 18 emotions, same temporal damping rules.

---

## Computed metrics

| Indicator | Formula (abridged) | Range |
|---|---|---|
| **Blood Pressure** | Estimate from PPI + age | 90/60 — 180/110 |
| **VO2 Max** | Tanaka + resting HR correction | 20-60 ml/kg/min |
| **Bio Age** | Chronological age ± HRV/HR/sleep deviation (requires ≥7 days) | ±10 years |
| **Coherence** | PPI power spectrum (0.04-0.26 Hz peak) | 0.0-1.0 |
| **Training Load** | TRIMP × duration × HR zones | capped 120min/200 TRIMP |
| **Sleep Score** | Phase distribution + duration + efficiency | 0-100 |

---

## API Reference (108 routes)

See RU version for the full grouped table. All endpoints under `/api/v1/`, require `Authorization: Bearer <token>` except health/docs.

Groups: Auth (4) · Users (3) · Biometrics (25) · WVI (9) · Emotions (8) · Activities (10) · Sleep (7) · AI (7) · Reports (5) · Alerts (6) · Device (6) · Training (4) · Risk (5) · Dashboard (3) · Export (3) · Settings (2) · Audit (1) · Social (4) · Health (4) · Docs (1) · Metrics (1).

---

## Infrastructure

### `docker-compose.yml` — 8 services

| Service | Image | Port | Purpose |
|---|---|---|---|
| **api** | Built from Dockerfile | 8091 (host network) | Rust API |
| **db** | postgres:16 | 5433 | Main DB |
| **redis** | redis:7 | 6379 | Cache |
| **kong** | kong/kong-gateway:latest | 8000/8001 | API Gateway |
| **prometheus** | prom/prometheus | 9090 | Metrics scraping |
| **grafana** | grafana/grafana | 3000 | Dashboards |
| **zookeeper** | confluentinc/cp-zookeeper | 2181 | Kafka coordinator |
| **kafka** | confluentinc/cp-kafka | 9092 | Event bus (5 topics) |

### Kubernetes (`kubernetes/`)
- `deployment.yaml` — replicas 3, resource limits
- `hpa.yaml` — HPA 3→100 on CPU (70%) / memory (80%)
- `service.yaml` — ClusterIP
- `ingress.yaml` — nginx ingress (TLS termination pending `api.wellex.ai`)
- `secrets.yaml` — DB password, OpenRouter key, Privy secrets

### Redis Cluster (`redis-cluster/`)
- 6-node cluster (3 masters + 3 replicas), sharded by user_id hash

### ArgoCD GitOps (`argocd/`)
- `application.yaml` — auto-sync from `master` branch
- `install.sh` — one-shot Helm install

### Kong declarative config (`kong/kong.yml`)
- Rate limiting: 60 req/min (authed), 20 req/min (anon)
- CORS whitelist, security headers, 5MB body limit, bot detection

---

## Observability

### Prometheus metrics (`/metrics` endpoint)
- `wvi_api_requests_total{method,path,status}`
- `wvi_api_request_duration_seconds{path}`
- `wvi_api_wvi_calculations_total`
- `wvi_api_emotion_detections_total{primary}`
- `wvi_api_active_users`

### Kafka event bus (5 topics)
- `biometrics.ingested`, `emotions.detected`, `wvi.calculated`, `alerts.fired`, `audit.logged`

### Structured JSON logging via `tracing` crate.

---

## Security hardening

- Per-user rate limiting (tower-governor 60/20 req/min)
- Request body limit 5MB (Kong + app)
- CORS whitelist, security headers, HSTS
- Audit logging (all auth/settings/sync actions)
- Graceful shutdown (SIGTERM drains requests + Kafka)
- K8s liveness/readiness probes
- Secrets via `kubernetes/secrets.yaml` + env vars
- SSL pinning on client (iOS) against zrok.io

---

## Setup / Run / Deploy

### Requirements
- Rust 1.88+ (`rustup install 1.88`)
- Docker 24+ with compose plugin
- `sqlx-cli` for migrations: `cargo install sqlx-cli`

### Local development

```bash
git clone https://github.com/alexmamasidikov-code/wvi-api-rust.git
cd wvi-api-rust
docker compose up -d db redis
DATABASE_URL=postgres://wellex:password@localhost:5433/wvi-db sqlx migrate run
cargo run
```

### Tests

```bash
cargo test --bin wvi-api                     # all
cargo test --bin wvi-api wvi::calculator     # specific module
RUST_LOG=debug cargo test --bin wvi-api -- --nocapture
```

### Full Docker stack

```bash
docker compose up -d               # 8 services
docker compose logs -f api
docker compose down
```

### Production deploy (100.90.71.111)

```bash
ssh root@100.90.71.111
cd /opt/wvi-api-rust
git pull origin master
docker compose up -d --build api
curl https://6ssssdj5s38h.share.zrok.io/api/v1/health/server-status
```

### Kubernetes

```bash
kubectl create namespace wvi
kubectl apply -f kubernetes/secrets.yaml
kubectl apply -f kubernetes/deployment.yaml
kubectl apply -f kubernetes/service.yaml
kubectl apply -f kubernetes/ingress.yaml
kubectl apply -f kubernetes/hpa.yaml
kubectl -n wvi get pods
kubectl -n wvi logs -l app=wvi-api -f
```

### ArgoCD (GitOps)

```bash
./argocd/install.sh
kubectl apply -f argocd/application.yaml
kubectl -n argocd get applications
```

### Load testing

```bash
brew install k6
k6 run loadtest.k6.js
```

Current baseline: **p95 = 208ms** at 1000 RPS.

---

## Related repositories

- **iOS client:** [alexmamasidikov-code/wvi-health-ios](https://github.com/alexmamasidikov-code/wvi-health-ios)
- **This repo (API):** [alexmamasidikov-code/wvi-api-rust](https://github.com/alexmamasidikov-code/wvi-api-rust)

---

**Last updated:** 2026-04-16 · **Commit:** `34e5418` (15 unit tests for WVI + Emotion)
