# WVI API — Wellness Vitality Index

High-performance Rust backend for the Wellex WVI health scoring platform.

Built with **Axum** + **Tokio** + **SQLx** + **PostgreSQL**.

## Architecture

```
115 API endpoints | 17 modules | 18 emotions | 64 activity types | 10 WVI metrics
```

### Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust 1.94+ |
| Framework | Axum 0.8 |
| Runtime | Tokio (async) |
| Database | PostgreSQL + SQLx |
| Auth | JWT (jsonwebtoken) + Argon2 |
| AI | Claude API (reqwest) |
| Docs | OpenAPI 3.1 (utoipa) |

## Quick Start

```bash
# Prerequisites: Rust, PostgreSQL

# Clone
git clone https://github.com/alexmamasidikov-code/wvi-api-rust.git
cd wvi-api-rust

# Setup database
createdb wvi
cp .env.example .env
# Edit .env with your DATABASE_URL

# Run migrations & start
cargo run
# Server starts on http://localhost:8091
```

## API Endpoints (115 total)

### Auth (3)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/auth/register` | Register new user |
| POST | `/api/v1/auth/login` | Login with credentials |
| POST | `/api/v1/auth/refresh` | Refresh access token |

### Users (4)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/users/me` | Get current user profile |
| PUT | `/api/v1/users/me` | Update user profile |
| GET | `/api/v1/users/me/norms` | Get personal biometric baselines |
| POST | `/api/v1/users/me/norms/calibrate` | Recalibrate personal norms |

### Biometrics (18)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/biometrics/sync` | Bulk sync biometric data |
| GET/POST | `/api/v1/biometrics/heart-rate` | Heart rate data |
| GET/POST | `/api/v1/biometrics/hrv` | HRV + stress + blood pressure |
| GET/POST | `/api/v1/biometrics/spo2` | Blood oxygen saturation |
| GET/POST | `/api/v1/biometrics/temperature` | Body temperature |
| GET/POST | `/api/v1/biometrics/sleep` | Sleep data |
| GET/POST | `/api/v1/biometrics/ppi` | PPI intervals |
| GET/POST | `/api/v1/biometrics/ecg` | ECG raw data |
| GET/POST | `/api/v1/biometrics/activity` | Activity data |
| GET | `/api/v1/biometrics/blood-pressure` | Blood pressure history |
| GET | `/api/v1/biometrics/stress` | Stress history |
| GET | `/api/v1/biometrics/breathing-rate` | Breathing rate |
| GET | `/api/v1/biometrics/rmssd` | RMSSD (HRV metric) |
| GET | `/api/v1/biometrics/coherence` | PPI coherence |
| GET | `/api/v1/biometrics/realtime` | Real-time snapshot |
| GET | `/api/v1/biometrics/summary` | Daily summary |

### WVI — Wellness Vitality Index (10)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/wvi/current` | Current WVI score with breakdown |
| GET | `/api/v1/wvi/history` | WVI history (time-series) |
| GET | `/api/v1/wvi/trends` | Trend analysis (7d/30d) |
| GET | `/api/v1/wvi/predict` | Predict WVI for next 6h |
| POST | `/api/v1/wvi/simulate` | Simulate WVI with changed inputs |
| GET | `/api/v1/wvi/circadian` | Circadian rhythm pattern |
| GET | `/api/v1/wvi/correlations` | Metric correlations |
| GET | `/api/v1/wvi/breakdown` | 10-metric score breakdown |
| GET | `/api/v1/wvi/compare` | Compare two time periods |

### Emotions (8)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/emotions/current` | Current detected emotion |
| GET | `/api/v1/emotions/history` | Emotion history |
| GET | `/api/v1/emotions/wellbeing` | Emotional wellbeing score |
| GET | `/api/v1/emotions/distribution` | Emotion distribution (24h) |
| GET | `/api/v1/emotions/heatmap` | Emotion heatmap by hour |
| GET | `/api/v1/emotions/transitions` | Emotion transition matrix |
| GET | `/api/v1/emotions/triggers` | Emotion triggers analysis |
| GET | `/api/v1/emotions/streaks` | Positive/negative streaks |

### Activities (10)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/activities/current` | Current detected activity |
| GET | `/api/v1/activities/history` | Activity history |
| GET | `/api/v1/activities/load` | Training load (TRIMP) |
| GET | `/api/v1/activities/zones` | HR zone distribution |
| GET | `/api/v1/activities/categories` | Activity categories summary |
| GET | `/api/v1/activities/transitions` | Activity transitions |
| GET | `/api/v1/activities/sedentary` | Sedentary behavior analysis |
| GET | `/api/v1/activities/exercise-log` | Exercise log |
| GET | `/api/v1/activities/recovery-status` | Recovery status |
| POST | `/api/v1/activities/manual-log` | Manual activity log |

### Sleep (7)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/sleep/last-night` | Last night's sleep analysis |
| GET | `/api/v1/sleep/score-history` | Sleep score history |
| GET | `/api/v1/sleep/architecture` | Sleep architecture (phases) |
| GET | `/api/v1/sleep/consistency` | Sleep consistency score |
| GET | `/api/v1/sleep/debt` | Sleep debt tracking |
| GET | `/api/v1/sleep/phases` | Sleep phase breakdown |
| GET | `/api/v1/sleep/optimal-window` | Optimal sleep window |

### AI — Claude Integration (7)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/ai/interpret` | AI interpretation of metrics |
| POST | `/api/v1/ai/recommendations` | Personalized recommendations |
| POST | `/api/v1/ai/chat` | Conversational AI health coach |
| POST | `/api/v1/ai/explain-metric` | Explain a specific metric |
| POST | `/api/v1/ai/action-plan` | Generate action plan |
| POST | `/api/v1/ai/insights` | AI-powered insights |
| POST | `/api/v1/ai/genius-layer` | 8 expert perspectives |

### Reports (5)
| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/reports/generate` | Generate a report |
| GET | `/api/v1/reports/list` | List user's reports |
| GET | `/api/v1/reports/templates` | Available templates |
| GET | `/api/v1/reports/:id` | Get report by ID |
| GET | `/api/v1/reports/:id/download` | Download report file |

### Alerts (6)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/alerts/list` | All alerts |
| GET | `/api/v1/alerts/active` | Active (unacknowledged) alerts |
| GET | `/api/v1/alerts/settings` | Alert threshold settings |
| GET | `/api/v1/alerts/history` | Alert history |
| POST | `/api/v1/alerts/:id/acknowledge` | Acknowledge an alert |
| GET | `/api/v1/alerts/stats` | Alert statistics |

### Device (6)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/device/status` | Device status and battery |
| POST | `/api/v1/device/auto-monitoring` | Configure auto-monitoring |
| POST | `/api/v1/device/sync` | Trigger device sync |
| GET | `/api/v1/device/capabilities` | Device capabilities |
| POST | `/api/v1/device/measure` | Start manual measurement |
| GET | `/api/v1/device/firmware` | Firmware info |

### Training (4)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/training/recommendation` | Today's training recommendation |
| GET | `/api/v1/training/weekly-plan` | Weekly training plan |
| GET | `/api/v1/training/overtraining-risk` | Overtraining risk score |
| GET | `/api/v1/training/optimal-time` | Best time to train today |

### Risk (5)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/risk/assessment` | Overall health risk assessment |
| GET | `/api/v1/risk/anomalies` | Detected anomalies |
| GET | `/api/v1/risk/chronic-flags` | Chronic pattern flags |
| GET | `/api/v1/risk/correlations` | Risk factor correlations |
| GET | `/api/v1/risk/volatility` | Metric volatility analysis |

### Dashboard (3)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/dashboard/widgets` | Dashboard widget data |
| GET | `/api/v1/dashboard/daily-brief` | Morning daily brief |
| GET | `/api/v1/dashboard/evening-review` | Evening review |

### Export (3)
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/export/csv` | Export data as CSV |
| GET | `/api/v1/export/json` | Export data as JSON |
| GET | `/api/v1/export/health-summary` | Export PDF health summary |

### Settings (4)
| Method | Path | Description |
|--------|------|-------------|
| GET/PUT | `/api/v1/settings` | App settings |
| GET/PUT | `/api/v1/settings/notifications` | Notification preferences |

### Health (3) — Public, no auth
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/health/server-status` | Server health check |
| GET | `/api/v1/health/api-version` | API version info |
| GET | `/api/v1/docs.json` | OpenAPI specification |

---

## WVI Algorithm

The WVI score (0-100) is computed from **10 weighted biometric metrics**:

| Metric | Weight | Source |
|--------|--------|--------|
| HRV | 0.18 | Heart Rate Variability (ms) |
| Stress | 0.15 | SDK stress index (inverted) |
| Sleep | 0.13 | Composite: deep%, duration, continuity |
| Emotional Wellbeing | 0.12 | 24h emotion history |
| SpO2 | 0.09 | Blood oxygen saturation |
| Heart Rate | 0.09 | Delta from resting HR |
| Activity | 0.08 | Steps + active minutes + METS |
| Blood Pressure | 0.06 | Deviation from 120/80 |
| Temperature | 0.05 | Deviation from personal baseline |
| PPI Coherence | 0.05 | Pulse-to-pulse interval regularity |

### Adaptive Weights
Weights change based on:
- **Time of day**: Night emphasizes sleep/temperature; morning emphasizes HRV/recovery; workday emphasizes stress
- **Exercise state**: During exercise, SpO2 and activity weights increase; HR weight decreases

### Emotion Feedback Loop
The detected emotion modifies the final WVI:
- Positive emotions (Flow, Meditative, Joyful): up to +12%
- Negative emotions (Exhausted, Fearful, Angry): up to -15%

## Emotion Engine

Detects **18 emotional states** from biometric signals using **Fuzzy Logic Cascade**:

### Positive (5)
- Calm, Relaxed, Joyful, Energized, Excited

### Neutral/Productive (4)
- Focused, Meditative, Recovering, Drowsy

### Negative (7)
- Stressed, Anxious, Angry, Frustrated, Fearful, Sad, Exhausted

### Physiological (2)
- Pain, Flow

Each emotion is scored using sigmoid and bell curve functions applied to biometric inputs (HR, HRV, stress, SpO2, temperature, BP, PPI coherence). Temporal smoothing prevents rapid switching (requires 30% confidence advantage within 5 minutes).

## Activity Detection

Recognizes **64 activity types** across 12 categories:
- Sleep (5): deep, light, REM, nap, falling asleep
- Rest (7): resting, sitting relaxed/working, standing, lying awake, phone scrolling, watching screen
- Walking (5): stroll, normal, brisk, hiking, nordic walking
- Running (5): jogging, tempo, interval, sprinting, trail
- Cardio Machine (4): cycling, stationary bike, elliptical, rowing
- Strength (5): weight training, bodyweight, CrossFit, HIIT, circuit
- Mind-Body (5): vinyasa yoga, hot yoga, pilates, stretching, meditation
- Sports (8): football, basketball, tennis, badminton, swimming, martial arts, dancing
- Daily (6): housework, cooking, driving, commuting, shopping, eating
- Physiological (7): stress event, panic attack, crying, laughing, pain, illness, intimacy
- Recovery (4): warm-up, cool-down, active recovery, passive recovery
- Mental (4): deep work, presentation, exam, creative flow

## Database Schema

17 PostgreSQL tables with indexes for time-series queries:
- `users`, `personal_norms`
- `heart_rate`, `hrv`, `spo2`, `temperature`, `sleep_records`, `ppi`, `ecg`, `activity`
- `wvi_scores`, `emotions`
- `alerts`, `alert_settings`, `reports`
- `app_settings`, `notification_settings`, `devices`

## Project Structure

```
src/
├── main.rs              — Axum router (115 routes)
├── config.rs            — Environment configuration
├── error.rs             — Error types + HTTP responses
├── auth/                — JWT auth + Argon2 passwords
├── users/               — User profiles + personal norms
├── biometrics/          — 18 biometric data endpoints
├── wvi/
│   ├── calculator.rs    — WVI score engine (adaptive weights)
│   ├── normalizer.rs    — Raw metrics → 0-100 normalization
│   ├── models.rs        — WVISnapshot, MetricScores, MetricWeights
│   └── handlers.rs      — 10 WVI endpoints
├── emotions/
│   ├── engine.rs        — 18-emotion fuzzy logic cascade
│   ├── models.rs        — EmotionState enum, EmotionResult
│   └── handlers.rs      — 8 emotion endpoints
├── activities/          — 64 activity type detection
├── sleep/               — Sleep analysis + debt tracking
├── ai/                  — Claude API integration
├── reports/             — Report generation
├── alerts/              — Health alert system
├── device/              — Wearable device management
├── training/            — Training recommendations
├── risk/                — Health risk assessment
├── dashboard/           — Dashboard widgets
├── export/              — CSV/JSON/PDF export
├── settings/            — App + notification settings
└── health/              — Public health checks
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgres://wvi:wvi@localhost:5432/wvi` | PostgreSQL connection |
| `JWT_SECRET` | — | JWT signing secret (required) |
| `JWT_EXPIRY_HOURS` | `24` | Token expiry in hours |
| `PORT` | `8091` | Server port |
| `CLAUDE_API_KEY` | — | Anthropic API key for AI features |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Claude model to use |

## License

Proprietary — Wellex Health

---

Built with Rust for maximum performance and reliability.
