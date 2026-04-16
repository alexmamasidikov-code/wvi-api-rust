# WVIHealth Architecture

## Overview

WVIHealth is a real-time wellness monitoring platform with:
- **iOS App**: SwiftUI + BLE bracelet integration (JCV8/V8 SDK)
- **Rust API**: Axum web server, 119+ endpoints, PostgreSQL
- **WVI v2 Algorithm**: 12-metric weighted geometric mean with progressive curve

## Components

### iOS App (69 Swift files)
- `WellexDeviceManager` — BLE connection, data streaming
- `EmotionEngine` — Local fuzzy logic (18 emotions)
- `BiometricCache` — Offline data persistence
- `DashboardViewModel` — WVI v2 local calculation

### Rust API (119 endpoints)
- `biometrics` — HR, HRV, SpO2, temperature, activity ingestion
- `wvi` — WVI v2 calculator (geometric mean, progressive curve, hard caps)
- `emotions` — Fuzzy logic engine (sigmoid/bell curves, 18 states)
- `ai` — OpenRouter integration (Gemini Flash)
- `social` — Feed, challenges, leaderboard

### Database (PostgreSQL)
- `users` — Authentication
- `heart_rate`, `hrv`, `spo2`, `temperature`, `activity` — Time-series biometrics
- `wvi_scores` — Calculated WVI scores
- `emotions` — Detected emotional states
- `sleep_records` — Sleep tracking data

## Data Flow

```
JCV8 Bracelet → BLE → iOS App → REST API → PostgreSQL
                        ↓                      ↓
                  Local WVI calc        Server WVI calc
                  EmotionEngine         EmotionEngine
                  BiometricCache        AI Insights
```

## Deployment

```bash
# Development
cargo run

# Docker
docker compose up -d

# Production
# See DEPLOY.md
```
