# Wellex Cycle Tracking — Production Implementation Plan

## Context

Пользовательницы Wellex (gender=female) хотят, чтобы браслет JCV8 автоматически отслеживал менструальный цикл, овуляцию и задержки без ручного ввода. На текущий момент функция отсутствует в обоих стэках (iOS WVIHealth и Rust API wvi-api-rust). Производитель браслета имеет в SDK enum-плейсхолдеры `setMenstruationInfo_V8 = 78` и `setPregnancyInfo_V8 = 79`, но публичные API-методы не реализованы — это ручная фича в будущем, не автодетекция.

При этом V8 SDK выдаёт сырые сигналы, достаточные для алгоритмического детектора уровня Oura/Apple Cycle Tracking:
- **Skin temperature** (ночная, NTC, 0.1°C resolution)
- **HRV** (RMSSD/SDNN, измеряется по 60-секундному окну покоя)
- **Resting HR** (continuous, downsampled)
- **Sleep architecture** (deep/light/REM/wake)

**Цель MVP:** автодетекция фазы цикла, прогноз менструации, ретроспективный детект овуляции, алерт задержки и PMS-предсказание с честным confidence score. Целевые метрики (взяты из validated peer-reviewed studies — Apple Hum Reprod 2025, Oura JMIR 2025, WHOOP npj Digital Medicine 2024):

| Метрика | Целевая точность v1 | Целевая точность v2 (после калибровки) |
|---|---|---|
| Retrospective ovulation ±2 дня | 80%+ | 85%+ |
| Forward ovulation prediction ±2 дня | 65–75% (regular) / 40–55% (irregular) | улучшение по мере накопления данных |
| Period prediction ±2 дня | 75–85% | 80%+ post 3-cycle calibration |
| Delay detection (binary, late >2 дня) | >95% | >95% |

**Зафиксированные решения из brainstorm:**
- **Режим:** auto + опциональный лог симптомов
- **MVP scope:** 8 фич (period prediction, retrospective ovulation, fertile window forecast, delay detection, phase ring, symptom logging, PMS prediction, period day tracking). Pregnancy mode и medical insights — Phase 2
- **Push:** 4 по умолчанию (fertile window / period coming / delay / PMS), 1 opt-in (cycle insight)
- **Cold-start:** Progressive blending — calendar predictor с дня 1, sensor model догоняет за 30–45 дней
- **Алгоритмический движок:** Hybrid (CUSUM + Bayesian fusion + multi-signal voting), готовый к ML-апгрейду
- **UI placement:** Cycle карточка в Body screen → Cycle detail screen с 4 внутренними табами
- **Onboarding:** новый шаг GenderSelection ДО Personalize; для female → CycleWelcome → ContraceptionMode → PeriodAnchor

**КРИТИЧЕСКИЕ изменения после deep research (отсутствовали в первой версии плана):**
1. **Hormonal contraception branch** — пользовательницы на гормональной контрацепции (КОК, гормональная ВМС, имплант, патч) НЕ имеют temperature shift. Все ovulation features для них отключаются. Это компетитор-стандарт (Oura, Apple, NC) и снижает legal-риск.
2. **PCOS / persistent anovulation escape hatch** — ~10% женщин имеют PCOS; алгоритм не находит ovulation 3+ цикла подряд → in-app сообщение с рекомендацией консультации врача
3. **General wellness positioning** — НЕ называем "fertile window" "контрацептивным средством"; добавляем явный disclaimer в каждой UI-точке: «Wellex Cycle Insights is not a medical device, not a contraceptive»
4. **Russia 152-ФЗ Art 18(5) compliance** — данные RU-граждан должны храниться в РФ; нужна изоляция в RU-резидентном шарде Postgres
5. **GDPR Art 35 DPIA** — обязательно перед запуском (health data + scale)

---

## High-Level Architecture

```
JCV8 Bracelet (BLE, V8 SDK)
   ↓ (existing)
iOS LiveMetricsHub (Core/LiveMetricsHub.swift)
   ↓ POST /api/v1/biometrics/sync (existing handler)
Rust biometrics::handlers::sync (87K LOC reference)
   ↓ Kafka event "wvi.biometrics" (existing event bus)
src/cycle/detector subscriber (NEW)
   ↓ writes cycle_phases, cycle_predictions, ovulation_events
   ↓
src/cycle/notifications (cron-driven, NEW)
   ↓ APNs via existing src/push/apns.rs
iOS PushNotificationManager
   ↓ deeplink wellex://cycle/today
WellexShellView.onOpenURL → NavigationCoordinator
   ↓
CycleHomeView (4-tab detail screen)
```

**Гейтинг:**
- **Backend:** все cycle endpoints возвращают 404 если `users.gender != 'female'`. Дополнительный DB-level check через `cycle_profiles.tracking_enabled` (можно отключить из Settings без потери данных).
- **iOS:** `SecureStorage.load("userSex") == "female"` + наличие cycle_profiles row → условный рендер карточки в Body screen и cycle complications.

**Hormonal contraception сценарий:**
- При онбординге собирается `contraception_method` (none / pill / hormonal_iud / implant / patch / ring / non_hormonal)
- Если non-hormonal или none → полный auto-flow с ovulation детектом
- Если hormonal → отключаются все ovulation/fertile-window features; показываем только period/symptom tracking без temperature claim

**Anovulation escape hatch:**
- Если 3 цикла подряд algorithm не находит sustained 0.2°C shift над 3 nights → одноразовое сообщение (без navi-блока в каждом запуске):
  > «За последние 3 цикла мы не зафиксировали типичный паттерн овуляции. Это бывает у 10–13% женщин (PCOS, перименопауза, стресс) и может быть нормой. Рекомендуем консультацию врача-гинеколога для персональной оценки.»
- Не блокирует фичу — продолжаем показывать period prediction по календарю; ovulation prediction скрыт.

---

## Backend (Rust) — `wvi-api-rust/src/cycle/`

### Module structure (~3500 LOC, ~25 файлов)

```
src/cycle/
├── mod.rs                       # public API + register routes via Router::new()
├── routes.rs                    # 12 HTTP endpoints (см. ниже)
├── models.rs                    # SQLx FromRow + serde DTO
├── events.rs                    # Kafka subscriber для wvi.biometrics
├── detector/
│   ├── mod.rs
│   ├── temperature.rs           # CUSUM + Marshall threshold на ночной skin temp
│   ├── hrv_rhr.rs               # luteal phase signal (HRV drop, RHR rise)
│   ├── sleep_signal.rs          # luteal sleep fragmentation как weak feature
│   ├── confidence.rs            # Bayesian fusion → confidence 0..1
│   ├── ovulation.rs             # композитное решение
│   └── outliers.rs              # ±2 SD rejection, illness/alcohol/jet-lag detection
├── predictor/
│   ├── mod.rs
│   ├── calendar.rs              # 28-day model + manual anchor + cycle history
│   ├── sensor.rs                # на основе detected ovulation
│   ├── blender.rs               # α-blend по дням данных
│   └── hormonal_branch.rs       # короткий-цикл прогноз для hormonal contraception users
├── ground_truth/
│   ├── labeler.rs               # собирает labels из period_logs для будущего ML
│   └── consistency.rs           # cross-check sensor vs self-report
├── insights/
│   ├── pms.rs                   # PMS-паттерн (HRV drop + RHR rise + sleep degrade в late luteal)
│   ├── narrator.rs              # AI-промпты для cycle-aware copy через src/ai/handlers.rs
│   └── anomaly.rs               # cycle-correlated сигнал в src/sensitivity/
├── notifications.rs             # 5 push categories, TZ-aware scheduling
├── lifecycle.rs                 # state machine: cold_start → calibrating → active → anovulatory → contraception
├── feature_flag.rs              # rollout gating (env var WELLEX_CYCLE_ENABLED + per-user opt-in)
└── tests/
    ├── fixtures/
    │   ├── regular_cycles.json     # 12 синтетических циклов с known ovulation
    │   ├── pcos_cycles.json        # anovulatory pattern
    │   ├── perimenopause.json      # variable-length cycles
    │   ├── postpartum.json         # 6-month amenorrhea + return
    │   └── hormonal_contraception.json  # suppressed shift
    ├── detector_tests.rs           # unit tests против fixtures
    ├── predictor_tests.rs
    ├── confidence_tests.rs
    └── integration_tests.rs        # end-to-end через biometrics ingest
```

### Database — Migration 018

`migrations/018_cycle_tracking.sql`:

```sql
-- User-level cycle profile and settings
CREATE TABLE cycle_profiles (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tracking_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    contraception_method VARCHAR(40) NOT NULL DEFAULT 'unknown',
        -- enum: 'none', 'pill', 'hormonal_iud', 'implant', 'patch', 'ring',
        -- 'copper_iud', 'condom_only', 'fam', 'unknown'
    is_pregnant BOOLEAN NOT NULL DEFAULT FALSE,
    is_breastfeeding BOOLEAN NOT NULL DEFAULT FALSE,
    is_perimenopause BOOLEAN NOT NULL DEFAULT FALSE,
    avg_cycle_length_days INT,           -- nullable, populated after 3 cycles
    avg_period_length_days INT,
    avg_luteal_length_days INT DEFAULT 14,
    last_anchor_date DATE,                -- last self-reported period start
    last_anchor_source VARCHAR(20),       -- 'onboarding'|'app'|'sensor'
    onboarded_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consent_special_category BOOLEAN NOT NULL DEFAULT FALSE,
    consent_special_category_at TIMESTAMPTZ
);

-- Detected/logged cycle events
CREATE TABLE cycles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cycle_number INT NOT NULL,             -- 1, 2, 3... per user
    start_date DATE NOT NULL,              -- period start (day 1)
    end_date DATE,                          -- start of next cycle - 1
    cycle_length_days INT,
    ovulation_date DATE,
    ovulation_confidence REAL CHECK (ovulation_confidence >= 0 AND ovulation_confidence <= 1),
    ovulation_method VARCHAR(20),          -- 'sensor'|'calendar'|'manual'|null
    luteal_length_days INT,
    notes TEXT,
    is_anovulatory BOOLEAN NOT NULL DEFAULT FALSE,
    detection_metadata JSONB,              -- {temp_shift_c, hrv_drop_ms, rhr_rise_bpm, day_count_above_baseline, ...}
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, cycle_number),
    UNIQUE(user_id, start_date)
);

-- Daily phase computation
CREATE TABLE cycle_phases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    phase VARCHAR(20) NOT NULL,            -- 'menstrual'|'follicular'|'ovulatory'|'luteal'|'unknown'
    cycle_day INT,                         -- day-of-cycle, 1..N
    cycle_id UUID REFERENCES cycles(id),
    confidence REAL,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_cycle_phases_user_date ON cycle_phases(user_id, date DESC);

-- Forward predictions
CREATE TABLE cycle_predictions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    predicted_period_start DATE,
    predicted_period_window_days INT NOT NULL DEFAULT 2,
    predicted_ovulation DATE,
    predicted_ovulation_window_days INT NOT NULL DEFAULT 2,
    predicted_fertile_start DATE,           -- ovulation - 5 days
    predicted_fertile_end DATE,             -- ovulation + 1 day
    period_confidence REAL,
    ovulation_confidence REAL,
    blender_alpha REAL NOT NULL,            -- 0..1, доля sensor-модели
    method_breakdown JSONB,                 -- {calendar: {weight, predicted_date}, sensor: {...}}
    valid_from DATE NOT NULL,
    valid_until DATE NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_cycle_predictions_user_valid ON cycle_predictions(user_id, valid_until DESC);

-- Period self-logs (manual or auto-detected)
CREATE TABLE period_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    flow_intensity VARCHAR(20),            -- 'spotting'|'light'|'medium'|'heavy'
    is_first_day BOOLEAN NOT NULL DEFAULT FALSE,
    is_last_day BOOLEAN NOT NULL DEFAULT FALSE,
    source VARCHAR(20) NOT NULL DEFAULT 'manual', -- 'manual'|'sensor_inferred'
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_period_logs_user_date ON period_logs(user_id, date DESC);

-- Symptoms (richness for ML training labels)
CREATE TABLE symptom_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    cramps INT CHECK (cramps >= 0 AND cramps <= 5),
    mood VARCHAR(20),                       -- 'low'|'neutral'|'good'|'irritable'|'anxious'
    libido VARCHAR(10),                     -- 'low'|'normal'|'high'
    headache BOOLEAN DEFAULT FALSE,
    bloating BOOLEAN DEFAULT FALSE,
    breast_tenderness BOOLEAN DEFAULT FALSE,
    spotting BOOLEAN DEFAULT FALSE,
    cervical_mucus VARCHAR(20),             -- 'dry'|'sticky'|'creamy'|'egg_white'|'watery'
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_symptom_logs_user_date ON symptom_logs(user_id, date DESC);

-- Per-night derived cycle signals (for detector)
CREATE TABLE cycle_signals_nightly (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    night_date DATE NOT NULL,                  -- date of "evening of"
    sleep_hours REAL,
    skin_temp_mean_c REAL,
    skin_temp_baseline_c REAL,                 -- 6-day rolling mean prior to this night
    skin_temp_delta_c REAL,                    -- actual - baseline
    skin_temp_above_baseline BOOLEAN,          -- delta >= 0.2°C
    rhr_bpm REAL,
    rhr_baseline_bpm REAL,                     -- 14-day rolling
    rhr_delta_bpm REAL,
    hrv_rmssd_ms REAL,
    hrv_baseline_ms REAL,
    hrv_delta_ms REAL,
    cusum_temp_score REAL,                     -- cumulative deviation
    is_outlier BOOLEAN NOT NULL DEFAULT FALSE,
    outlier_reason VARCHAR(40),                -- 'high_alcohol'|'fever'|'jet_lag'|'low_sleep'|null
    coverage_pct REAL,                          -- % of night with valid sensor reads
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, night_date)
);
CREATE INDEX idx_cycle_signals_user_night ON cycle_signals_nightly(user_id, night_date DESC);

-- Audit / consent log (GDPR Art 7 evidentiary trail)
CREATE TABLE cycle_consent_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    consent_type VARCHAR(40) NOT NULL,         -- 'special_category_health'|'predictions'|'notifications'|'analytics'
    granted BOOLEAN NOT NULL,
    consent_text_version VARCHAR(20) NOT NULL, -- e.g. 'v1.0_2026-04-27'
    locale VARCHAR(10) NOT NULL,
    ip_address INET,
    device_fingerprint VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_cycle_consent_user ON cycle_consent_log(user_id, created_at DESC);
```

**Postgres notes:**
- Все TIMESTAMPTZ нормализуем к UTC (паттерн уже в проекте)
- DATE — в локальном TZ пользовательницы (нужно добавить колонку `users.timezone` если ещё нет)
- `ON DELETE CASCADE` для GDPR Art 17 right-to-erasure compliance
- Все UNIQUE индексы — для idempotent logging

### API Endpoints

| Method | Path | Auth | Назначение | Status if not female |
|---|---|---|---|---|
| POST | `/api/v1/cycle/onboarding` | Bearer | Initial setup: contraception, last period date, avg cycle length | 404 |
| GET | `/api/v1/cycle/profile` | Bearer | Read cycle_profiles row | 404 |
| PATCH | `/api/v1/cycle/profile` | Bearer | Update profile (e.g., contraception change, pregnancy toggle) | 404 |
| GET | `/api/v1/cycle/state` | Bearer | Today snapshot: phase, day, predictions, confidence, anovulatory flag | 404 |
| GET | `/api/v1/cycle/calendar?from=&to=` | Bearer | Phases + period_logs + predictions for date range | 404 |
| GET | `/api/v1/cycle/insights` | Bearer | Aggregates: avg cycle length, regularity %, temperature curves, AI narrator output | 404 |
| GET | `/api/v1/cycle/history` | Bearer | List of past cycles | 404 |
| POST | `/api/v1/cycle/period-log` | Bearer | Log a period day | 404 |
| DELETE | `/api/v1/cycle/period-log/:date` | Bearer | Remove a period log | 404 |
| POST | `/api/v1/cycle/symptom-log` | Bearer | Log symptoms | 404 |
| GET | `/api/v1/cycle/export` | Bearer | GDPR Art 20 data portability — JSON export of all cycle data | 404 |
| DELETE | `/api/v1/cycle/all-data` | Bearer | GDPR Art 17 erasure — wipes all cycle_* rows | 404 |
| POST | `/api/v1/cycle/admin/recompute` | Bearer (admin) | Force-rerun detector (QA / migration) | 404 |

**Auth:** существующий middleware `auth/middleware.rs` (Bearer + Privy JWT). Все cycle endpoints проверяют:
1. Валидный auth
2. `users.gender == 'female'` (запрос к users table)
3. `cycle_profiles.tracking_enabled == true` (для не-onboarding endpoints)

**Rate limiting:** writes (period-log, symptom-log) — стандартный 20/s. Reads — 100/s.

### Detection Algorithm — Spec

**Этап 0 — Outlier rejection** (`detector/outliers.rs`):
- Сон <4 часов → skip night
- Skin temp delta >2 SD от 14-day rolling SD → mark outlier
- High activity in 2h before sleep (>10k steps last hour) → mark outlier
- Если RHR >110 на ночь → flag fever, exclude
- Coverage <70% of night with valid temp reads → skip

**Этап 1 — Temperature CUSUM + Marshall threshold** (`detector/temperature.rs`):

Per-night signal:
```
baseline_t(d) = mean(skin_temp[d-7..d-1] excluding outliers)
delta(d) = skin_temp(d) - baseline_t(d)
above_threshold(d) = delta(d) >= 0.20°C  // Apple Hum Reprod 2025
```

Marshall biphasic detection:
```
ovulation_candidate(d) = above_threshold(d) AND above_threshold(d+1) AND above_threshold(d+2)
ovulation_estimate(d) = d - 1   // ovulation = day before sustained shift
```

CUSUM (для smoother detection of sustained shift):
```
S(d) = max(0, S(d-1) + delta(d) - 0.10°C)   // reference threshold
trigger when S(d) > 0.30°C
```

**Этап 2 — HRV/RHR luteal signal** (`detector/hrv_rhr.rs`):

Из WHOOP data (npj Digital Medicine 2024, n=11,590):
```
luteal_rhr_shift = +2.73 BPM mean (vs follicular)
luteal_hrv_shift = -4.65 ms RMSSD mean
```

Per-night:
```
rhr_baseline(d) = mean(rhr[d-14..d-1])
rhr_delta(d) = rhr(d) - rhr_baseline(d)
hrv_baseline(d) = mean(hrv_rmssd[d-14..d-1])
hrv_delta(d) = hrv_rmssd(d) - hrv_baseline(d)

luteal_signal(d) = (rhr_delta >= +1.5) AND (hrv_delta <= -2.0)  // conservative
```

Used as confirmation, not standalone detection (signal too noisy alone).

**Этап 3 — Bayesian fusion** (`detector/confidence.rs`):

```
P(ovulation_at_day_N | data) ∝
    P(temp_shift_at_N+1 | ovulation_N) ·         // 0.65 if shift detected, 0.20 otherwise
    P(luteal_signal_at_N+2..N+5 | ovulation_N) · // 0.30 if signal, 0.10 otherwise
    P(N | calendar_prior)                          // gaussian over expected day

normalized_confidence = P / sum(P over reasonable window)
```

Confidence thresholds:
- `≥ 0.8` → "high confidence" UI badge, push notifications enabled
- `0.5–0.8` → "medium" UI badge, predictions shown but not pushed
- `< 0.5` → "low" UI badge, predictions hidden, only "still calibrating"

**Этап 4 — Predictor blender** (`predictor/blender.rs`):

```rust
// Linear ramp from 0 to 1 over days 30..60 of data
let alpha = ((data_days - 30.0) / 30.0).clamp(0.0, 1.0);

let predicted_period = lerp(calendar_pred, sensor_pred, alpha);
let displayed_confidence = lerp(0.55, sensor_confidence, alpha);
```

If user has `is_anovulatory` flag → only calendar predictor used, ovulation hidden.
If user has `contraception_method ∈ {pill, hormonal_iud, implant, patch, ring}` → only "withdrawal bleed" calendar predictor (21-day cycle assumed, no ovulation).

### Edge case branches (`lifecycle.rs`)

State machine for `cycle_lifecycle`:

```
cold_start (0-29 days data)
  → calibrating (30-59 days) [α ramp 0→1]
  → active (60+ days, regular) [full sensor]
       → anovulatory (3 cycles no ovulation) [calendar-only + escape hatch message]
       → contraception (user changed contraception) [withdrawal bleed mode]
       → pregnancy (user toggled is_pregnant) [feature paused, weeks tracker shown — Phase 2]
       → perimenopause (user >45 + variability >7 days, 6+ cycles) [increased prediction window]
```

Transitions logged in `cycle_consent_log` with type `lifecycle_transition`.

### Existing infrastructure to reuse

- **Kafka event bus** (`src/events.rs`): subscribe to topic `wvi.biometrics`, filter for temperature/hrv/sleep records, run incremental detector update
- **TZ-aware cron pattern** (`src/narrator_schedule.rs`): copy `should_fire_morning(now_local)` for daily 07:00 cycle batch job; use `daily_brief_log`-style table `cycle_cron_log` for atomic dedup
- **Push pipeline** (`src/push/apns.rs`): existing send_alert + JWT cache; add 5 new categories CYCLE_FERTILE / CYCLE_PERIOD_COMING / CYCLE_DELAY / CYCLE_PMS / CYCLE_INSIGHT
- **AI prompt rules** (`src/ai/prompt_rules.rs`): existing skill #6 (Thermoregulation) уже упоминает menstrual cycle impacts. Расширить там же дополнительной cycle-specific guidance, не плодить новый prompt
- **AI precompute pattern** (`src/ai/precompute.rs`): добавить новый `AiEndpointKind::CycleNarrative` → daily AI insight для Today таба (cached 10 мин)
- **Sensitivity module** (`src/sensitivity/`): add cycle-correlated signals (e.g., "your stress is elevated, common in late luteal phase")
- **Audit log** (`src/audit.rs`): log every cycle_profiles update, every consent change

### Push notifications spec

| Category | Trigger | When | Confidence gate | TZ |
|---|---|---|---|---|
| CYCLE_FERTILE | Predicted ovulation date - 1 day | 09:00 local | ovulation_confidence ≥ 0.6 | per-user |
| CYCLE_PERIOD_COMING | Predicted period date - 2 days | 09:00 local | period_confidence ≥ 0.6 | per-user |
| CYCLE_DELAY | Today is predicted_period_start + 2 days AND no period log | 12:00 local | always | per-user |
| CYCLE_PMS | Predicted period date - 4 days, AND prior cycle showed PMS pattern (HRV drop, RHR rise, sleep degrade) | 18:00 local | requires 2 cycles of pattern | per-user |
| CYCLE_INSIGHT (opt-in) | After detected cycle end | 09:00 local next day | always | per-user |

Все push deeplink → `wellex://cycle/today` (открывает Today таб через NavigationCoordinator).
Дедупликация через `cycle_push_log(user_id, category, date_fired_utc, deep_local_date)` — UNIQUE constraint.
Token invalidation handled by existing 410 response → token removed from push_tokens table.

---

## iOS (WVIHealth)

### Step 1 — Onboarding extensions

**Файлы изменяются:**

`Features/Onboarding/OnboardingState.swift` — расширяем enum:
```swift
enum OnboardingStage: Equatable {
    // ... existing cases
    case genderSelection
    case cycleWelcome              // только если female
    case cycleContraception        // только если female
    case cyclePeriodAnchor         // только если female
}
```

`Features/Onboarding/OnboardingCoordinator.swift` — добавляем:
- `selectedGender: String?` state
- `selectedContraception: String?` state
- `lastPeriodDate: Date?` state (опционально)
- `avgCycleLength: Int?` state (опционально, default 28)
- Methods: `selectGender(_:)`, `selectContraception(_:)`, `setPeriodAnchor(_:_:)`, `skipPeriodAnchor()`
- В transitions: `genderSelection` → если female `cycleWelcome`, иначе `personalize`. `cycleWelcome` → `cycleContraception` → `cyclePeriodAnchor` → `personalize`.
- Сохранение: `SecureStorage.save("userSex", value)` + `UserDefaults.standard.set("wellex.profile.gender", value)` + POST `/api/v1/users/me` (gender). Cycle data → POST `/api/v1/cycle/onboarding`.

`Features/Onboarding/RootRouterView.swift` — добавляем 4 case'а:
```swift
case .genderSelection:        GenderSelectionView()
case .cycleWelcome:           CycleTrackingWelcomeView()
case .cycleContraception:     CycleContraceptionView()
case .cyclePeriodAnchor:      CyclePeriodAnchorView()
```

**Файлы создаются:**

`Features/Onboarding/Screens/GenderSelectionView.swift` — копия структуры `PersonalizeView.swift` (89 строк):
- BackButton + progress dots (4 dots, в onboarding indicator)
- Title: "Tell us about you"
- Sub: "We tailor metrics and features to your physiology"
- 3 карточки: Female / Male / Prefer not to say
- WellexButton "Continue"
- На continue → `coord.selectGender(value)`

`Features/Onboarding/Screens/CycleTrackingWelcomeView.swift` — приватность-first hero:
- BackButton + progress dots
- Hero illustration: 5 концентрических колец (использует `ScanRings` из SharedComponents.swift, recolored to cycle pillar #C026D3)
- Title: "Cycle understanding built into your body"
- Body: "Your bracelet's sensors detect ovulation, period, and delays automatically. No manual logging needed."
- Privacy block: «Cycle data is special-category health data under GDPR. We process it only with your explicit consent, store it encrypted at rest, and never share with third parties. You can export or delete it anytime.»
- Consent toggle (required to proceed): "I consent to processing of my cycle health data" → пишем в `cycle_consent_log` через `/api/v1/cycle/onboarding`
- WellexButton "Continue" (disabled until consent toggled)
- Внизу мелким: "Wellex Cycle Insights is not a medical device. It does not diagnose, treat, or prevent disease. It is not a contraceptive."

`Features/Onboarding/Screens/CycleContraceptionView.swift`:
- BackButton + progress dots
- Title: "Are you using hormonal contraception?"
- Sub: "Hormonal methods change how your body's signals work. We adjust accordingly."
- 5 cards: None or non-hormonal / Pill / Hormonal IUD / Implant / Other (Other → text field или skip)
- WellexButton "Continue" → POST `/api/v1/cycle/profile` (PATCH с contraception_method)
- На select hormonal-method → следующий экран показывает badge: "Ovulation predictions disabled (hormonal contraception affects sensor signals)"

`Features/Onboarding/Screens/CyclePeriodAnchorView.swift`:
- BackButton + progress dots
- Title: "When did your last period start?"
- Sub: "We use this to start predictions on day 1. Skip if not sure."
- DatePicker (compact, max 90 days back, default = 14 days ago)
- Optional: avg cycle length slider (21-35 days, default 28)
- "I don't remember" Ghost button → skip with population prior
- WellexButton "Continue" → POST `/api/v1/cycle/onboarding` с anchor

**Цвета и шрифты:**
- Background: `WVIColor.wlxVoid` (#08070F)
- Cards: `WVIColor.wlxCard` (white 0.05)
- Title: `Onest-ExtraLight 28pt`, `WVIColor.wlxText1`
- Sub: `Onest-Light 13pt italic`, `WVIColor.wlxText2`
- Body: `Onest-Light 14pt`, `WVIColor.wlxText2`
- Caption: `Onest-Light 11pt`, `WVIColor.wlxText3`
- Cycle accent: добавить в Theme.swift `static let cycle = Color(hex: 0xC026D3)` (deep berry, не конфликтует с emotionExcited #F472B6)
- Animation: `.wlxScene` для transitions, `.pressSpring` для CTAs

### Step 2 — Cycle card в Body screen

**Файл изменяется:**

`Features/Body/BodyScreen.swift`:
- Добавить state: `@State var cycleSummary: CycleSummary? = nil`
- Добавить trigger загрузки в onAppear: `await loadCycle()`

`Features/Body/BodyScreen+Sections.swift`:
- Добавить ViewBuilder `cycleSection`:
```swift
@ViewBuilder var cycleSection: some View {
    if isFemale && cyclePillarEnabled {
        if let summary = cycleSummary {
            CycleSummaryCard(summary: summary)
        } else {
            CycleEmptyCard()  // "Set up cycle tracking →"
        }
    }
}
private var isFemale: Bool {
    SecureStorage.load("userSex")?.lowercased() == "female"
}
```
- Расположить `cycleSection` после Sleep&Recovery card (визуальная парность с recovery)

**Файлы создаются:**

`Features/Body/Cards/CycleSummaryCard.swift`:
- GlassCard с двумя строками:
  - Левая часть: phase ring (RingProgress 64pt, color `WVIColor.cycle`, fill = cycle_day / cycle_length)
  - Правая часть: phase name (large) + "Day X of Y" + next event preview ("Period in 4 days · 78%")
- TapGesture → NavigationLink → `CycleHomeView`
- Skeleton при загрузке

`Features/Body/Cards/CycleEmptyCard.swift`:
- GlassCard с CTA "Set up cycle tracking" + accent ring stub
- Tap → re-launch onboarding для cycle (dialog или modal)

### Step 3 — Cycle detail screen с 4 табами

**Файлы создаются:**

`Features/Cycle/CycleHomeView.swift` — wrapper:
- Custom BackButton
- Title row: "Cycle" + accent dot
- Top segmented selector (custom, не TabView): Today / Calendar / Insights / History
- Below: соответствующий sub-view
- На каждом sub-view внизу опциональный AIInsightCard (cycle narrative из `/api/v1/cycle/insights`)

`Features/Cycle/Tabs/CycleTodayTab.swift`:
- Hero ring: phase + cycle day + confidence dot
- Below: 3 cards
  - "Next event" — predicted period or ovulation, с уверенностью
  - "Cycle health" — regularity badge (regular / variable / irregular), avg length
  - "Today's body" — temp delta vs baseline, HRV note, RHR note (без medical claims)
- 2 CTA внизу: "Log period" / "Log symptom"
- Lifecycle banners (only when relevant):
  - cold_start: "Calibrating — predictions improve with more nights worn"
  - anovulatory: escape hatch message (показать ОДИН раз, флаг в UserDefaults)
  - contraception: "You're on hormonal contraception. Ovulation predictions are off."

`Features/Cycle/Tabs/CycleCalendarTab.swift`:
- Month-view grid с цветными кружочками по фазам:
  - Menstrual: solid #C026D3
  - Follicular: light #C026D3 30%
  - Ovulatory: glow #C026D3 with star
  - Luteal: gradient
  - Predicted: striped pattern
- Tap day → bottom sheet с подробностями
- Symptom dots на logged days
- Period flow indicators
- Swipe для навигации month-to-month

`Features/Cycle/Tabs/CycleInsightsTab.swift`:
- Charts:
  - Avg cycle length (last 6 cycles, bar chart)
  - Regularity score (0-100, with badge)
  - Skin temp curve overlay (last cycle, with luteal phase shaded)
  - HRV by phase (box plot)
  - PMS pattern (когда применимо)
- AI narrative card: "Your last 3 cycles show stable luteal phases averaging 13 days..."

`Features/Cycle/Tabs/CycleHistoryTab.swift`:
- LazyVStack of past cycles
- Каждый row: cycle number, dates, length, ovulation date, regularity flag
- Tap → cycle detail bottom sheet с подробностями + symptom timeline

### Step 4 — Bottom sheets

`Features/Cycle/Sheets/PeriodLogSheet.swift`:
- DatePicker (default today)
- 4 flow buttons: Spotting / Light / Medium / Heavy
- Toggles: First day of period / Last day of period
- Save → POST `/api/v1/cycle/period-log`
- Haptic on save (Haptic.success)

`Features/Cycle/Sheets/SymptomLogSheet.swift`:
- DatePicker (default today)
- Cramps slider (0-5, with emoji)
- Mood: 5 chip buttons (low/neutral/good/irritable/anxious)
- Libido: 3 chips (low/normal/high)
- Toggles: headache / bloating / breast tenderness / spotting
- Cervical mucus picker: dry/sticky/creamy/egg_white/watery (with help icon)
- Notes field
- Save → POST `/api/v1/cycle/symptom-log`

### Step 5 — Settings integration

`Features/Settings/SettingsView.swift` — добавляем секцию "Cycle Tracking":
- @AppStorage flags: `cycleNotificationsEnabled`, `cycleFertileNotif`, `cyclePeriodNotif`, `cycleDelayNotif`, `cyclePmsNotif`, `cycleInsightNotif` (last is opt-in, default false)
- Sub-sections:
  - Master toggle "Enable cycle tracking" (off → backend `/api/v1/cycle/profile { tracking_enabled: false }`, не удаляет данные)
  - Notification toggles (4 default + 1 opt-in)
  - "Update contraception method" navigation
  - "Toggle pregnancy mode" (Phase 2; для MVP = disabled link)
  - "Export my cycle data" → POST `/api/v1/cycle/export` → JSON download (GDPR Art 20)
  - "Delete all cycle data" → confirm dialog → DELETE `/api/v1/cycle/all-data` (GDPR Art 17)

### Step 6 — Push notifications

`Core/Notifications/PushNotificationManager.swift` — расширяем:
- Добавить в `Category` enum:
```swift
static let cycleFertile = "CYCLE_FERTILE"
static let cyclePeriodComing = "CYCLE_PERIOD_COMING"
static let cycleDelay = "CYCLE_DELAY"
static let cyclePms = "CYCLE_PMS"
static let cycleInsight = "CYCLE_INSIGHT"
```
- Categories с действиями: View Details / Snooze 1 day / Dismiss
- В `userNotificationCenter(_:didReceive:)` — switch для cycle категорий → deeplink `wellex://cycle/today`
- В `WellexShellView.onOpenURL` — обрабатываем `wellex://cycle/today` → `NavigationCoordinator.selectedTab = .body` + push CycleHomeView

### Step 7 — Watch complication

`WVIHealthWatch/CycleComplicationProvider.swift` (NEW):
- Identifier: `cycle_phase`
- Families: `.graphicCircular`, `.graphicCorner`
- Reads from shared UserDefaults `group.com.wvi.health`:
  - `cache_cycle_phase` (string)
  - `cache_cycle_day` (int)
  - `cache_cycle_phase_color` (hex string)
- Template: graphicCircularStackText {phase short, day}
- Timeline policy: refresh every hour
- Sync from main app: `WatchConnectivity.shared.transferUserInfo(cycleData)` on every cycle update

### Step 8 — Widget

`WVIHealthWidget/CycleWidget.swift` (NEW):
- Widget kind `cycle_widget`
- Sizes: `.systemSmall`, `.systemMedium`
- Reads from shared UserDefaults same keys as complication
- Small: ring + phase short + day
- Medium: ring + phase + day + next event preview
- Stale threshold: 12 hours (cycle data changes slowly)
- Add to `@main` Widget bundle

### Step 9 — Analytics

`Core/Logging/AnalyticsEvent.swift` — добавляем:
```swift
case cycleOnboardingStarted
case cycleOnboardingCompleted(contraception: String, anchorProvided: Bool)
case cycleOnboardingSkipped
case cycleConsentToggled(granted: Bool)
case cycleHomeOpened(tab: String)
case cyclePhaseViewed(phase: String, day: Int)
case cyclePeriodLogged(intensity: String)
case cycleSymptomLogged(symptoms: [String])
case cyclePushDelivered(category: String)
case cyclePushTapped(category: String)
case cycleNotificationToggled(category: String, enabled: Bool)
case cycleAnomalyEscapeHatchShown
case cycleDataExported
case cycleDataDeleted
```
Category для всех: `"cycle"`

### Step 10 — Localization

`Core/Localization.swift` — добавляем 60+ строк, паттерн `t("EN", "RU")` (полный список — в отдельной задаче переводов на 6 языков EN/RU/FR/ES/PT-BR/ZH-Hans). Ключевые группы:
- Onboarding: gender_selection, cycle_welcome_title, cycle_consent_text, contraception_question, contraception_pill, contraception_iud, period_anchor_question, period_anchor_skip
- Tabs: today, calendar, insights, history
- Phases: menstrual, follicular, ovulatory, luteal, unknown
- Predictions: next_period, next_ovulation, fertile_window, days_until, confidence_high, confidence_medium, confidence_low
- Logs: log_period, log_symptom, flow_spotting, flow_light, flow_medium, flow_heavy, mood_low, mood_neutral, etc.
- Push: push_fertile_title, push_fertile_body, push_period_title, push_period_body, push_delay_title, push_delay_body, push_pms_title, push_pms_body
- Settings: cycle_section, cycle_notifications, cycle_export, cycle_delete_all, cycle_delete_confirm
- Disclaimers: not_medical_device, not_contraceptive, consult_clinician

Подрядчики переводов: внешние, бюджет ~50 строк × 6 языков × $0.20 = ~$60. Запустить параллельно с разработкой.

### Step 11 — Accessibility

- Каждое кольцо/график: `.accessibilityLabel("Cycle phase \(phase), day \(day) of \(length)")`
- Phase colors всегда продублированы текстом (а не только цветом)
- Calendar grid: `.accessibilityElement(children: .combine)` per day cell с описанием "April 15, follicular phase"
- Period log buttons: `.accessibilityHint("Logs your period day")`
- Reduce Motion: cycle ring анимация → fade-only вместо rotate
- Reduce Transparency: GlassCard fallback на solid background
- Dynamic Type: все text использует WVIFont tokens (scalable)
- VoiceOver test pass обязателен перед launch

---

## Compliance & Legal

### FDA general-wellness positioning

**Стратегия:** оставаться в Class I exempt (no FDA filing needed), positioning как "wellness tracker", не medical device.

**Что НЕЛЬЗЯ говорить в копи:**
- "FDA-approved", "FDA-cleared", "medical-grade"
- "Diagnose", "treat", "cure", "prevent disease"
- "Abnormal", "high risk", "consult immediately" (в pushах)
- "Contraceptive", "birth control", "use for pregnancy prevention"
- "Fertile day" без качественного словесного hedge ("estimated fertile window")

**Что МОЖНО:**
- "Track patterns", "general wellness", "understand your cycle"
- "Estimated ovulation day"
- "Recommend consulting a healthcare professional" (general, not specific to a finding)

**Required disclaimer (везде где появляется prediction):**
> «Wellex Cycle Insights is a general-wellness feature, not a medical device. It does not diagnose, treat, or prevent disease. It is not a contraceptive. Consult a healthcare professional for medical advice.»

Локации disclaimer:
- Cycle Welcome screen (полный текст)
- Cycle Today tab footer (короткая версия)
- Каждый push (footer line "Not medical advice")
- Settings → About cycle tracking (полный текст)
- Privacy policy section "Cycle data"

### GDPR Article 9 — special category health data

**Обязательные процессы:**
1. **Explicit consent** — toggle на CycleWelcome screen, log в `cycle_consent_log`. Withdrawable через Settings → Delete all cycle data.
2. **DPIA (Data Protection Impact Assessment)** — провести и записать ДО launch:
   - Lawful basis: Article 9(2)(a) explicit consent
   - Data minimization: только necessary fields, не собираем precise geolocation
   - Risk: re-identification если data leak, mitigation: encryption at rest + in transit, access logs
   - Документ DPIA сохранить в `/Users/alexander/Code/wvi-api-rust/docs/dpia/2026-04-cycle-tracking.md` для аудита
3. **Right to erasure (Art 17)** — DELETE `/api/v1/cycle/all-data` endpoint, на клиенте кнопка с confirm
4. **Right to portability (Art 20)** — GET `/api/v1/cycle/export` возвращает JSON со всеми данными
5. **Records of processing (Art 30)** — `cycle_consent_log` + `audit_log` хранят evidentiary trail
6. **Data residency** — для EU users, primary DB shard в EU region (Frankfurt). Cross-border to US through SCCs.

### Russia 152-ФЗ Article 18(5)

**Strict requirement:** для пользовательниц-граждан РФ, cycle data MUST первично writeться в DB на территории РФ.

**Текущее состояние Wellex:** все DB на CherryServers/dev-стенде в EU. Для compliance нужен план:
- Phase 1 (MVP): отложить onboarding для RU users до решения локализации (или геофенс через Russian users → no cycle features). Можно показать "Coming soon to Russia" message при ru-locale.
- Phase 2: развернуть RU-resident shard (Yandex Cloud Postgres), routing на gateway level по `users.country`.
- Phase 3: registrate Wellex как оператора персональных данных в Roskomnadzor (https://rkn.gov.ru) — обычно занимает 30 дней.

**Решение для MVP:** показать exclusion banner для пользовательниц с `users.country == 'RU'` или Russian device locale. Cycle tracking отключён до локализации DB.

### Apple App Store privacy

- В Privacy Nutrition Labels добавить категорию "Health & Fitness" → "Cycle Tracking"
- Privacy manifest (`PrivacyInfo.xcprivacy`): декларировать `NSPrivacyAccessedAPICategorySensitiveData` для cycle data
- Submit reviewer notes: "Cycle tracking is general wellness, not medical device. No contraceptive claims."

---

## Testing Strategy

### Unit tests (Rust)

`src/cycle/tests/`:
- `detector_tests.rs` — против fixture файлов:
  - `regular_cycles.json` (12 циклов с known LH-confirmed ovulation): expect ≥80% detection within ±2 days
  - `pcos_cycles.json` (anovulatory): expect 0 ovulation found, anovulatory flag set after 3 cycles
  - `perimenopause.json`: expect predictions widen window
  - `postpartum.json`: expect cycle detection only after 6+ months
  - `hormonal_contraception.json`: expect ovulation prediction disabled
- `confidence_tests.rs` — Bayesian fusion math:
  - Pure temp signal → confidence ≥0.8
  - Temp + HRV/RHR → confidence ≥0.9
  - No signal → confidence ≤0.3
- `predictor_tests.rs` — calendar vs sensor blending:
  - Day 0: α=0, prediction = calendar
  - Day 30: α=0, prediction = calendar
  - Day 45: α=0.5, prediction = lerp
  - Day 60+: α=1, prediction = sensor
- `outliers_tests.rs` — illness/jet-lag detection
- `lifecycle_tests.rs` — state transitions

### Unit tests (Swift)

`WVIHealthTests/`:
- `CycleViewModelTests.swift` — onboarding state, fetch state, log mutations
- `CycleAPIClientTests.swift` — endpoint construction, error handling
- `CyclePillarRingTests.swift` — phase mapping to color/label

### Integration tests

`/Users/alexander/Code/wvi-api-rust/tests/cycle_test.rs`:
- POST onboarding → GET state → POST period log → GET state (verify update)
- POST hormonal contraception → GET state (verify ovulation hidden)
- DELETE all-data → GET state (verify 404)
- Mock 60 days biometrics → daily batch → verify sensor predictor takes over
- Test gender gating: non-female user → 404 на всех cycle endpoints

### Snapshot tests (iOS)

- `CycleSummaryCardSnapshot.swift`: 5 phases × 3 confidence levels = 15 snapshots
- `CycleHomeViewSnapshot.swift`: 4 tabs × empty/loaded/error = 12 snapshots
- `CycleOnboardingSnapshot.swift`: 4 onboarding screens × 6 locales = 24 snapshots

### Beta cohort + LH validation

Перед public launch:
- 50 beta users, 2 cycles minimum nightly wear
- Optional: partnership с фертильной клиникой для 100-300 LH-validated cycles. Контакт: TBD (университет/clinic нужно идентифицировать)
- Metrics review до релиза:
  - retrospective ovulation accuracy (target ≥80%)
  - period prediction accuracy (target ≥80%)
  - false-positive PMS push rate (target <15%)
  - user retention week 4 (target ≥60%)

---

## Phasing & Rollout

### Phase 0 — Compliance prep (1 неделя)
- DPIA written and reviewed (legal: alex@crossfi.org)
- Privacy policy update with cycle section
- App Store reviewer notes prepared
- Localization vendor briefed (60 strings × 6 langs)
- RU exclusion banner copy

### Phase 1 — Backend foundation (2 недели)
- Migration 018, models, routes (calendar predictor only)
- onboarding endpoint, profile CRUD, state endpoint
- Calendar predictor with anchor
- Russian user exclusion at gateway
- GDPR Art 17/20 endpoints

**Exit criteria:** end-to-end onboarding via curl, calendar predictions return for non-RU female users.

### Phase 2 — iOS onboarding + Body card + Today tab (2 недели)
- 4 new onboarding screens (Gender, Welcome, Contraception, PeriodAnchor)
- Cycle card в BodyScreen
- CycleHomeView с одним табом Today
- Period/Symptom log sheets
- Settings cycle section
- Localization integrated

**Exit criteria:** female user can complete onboarding, see phase on Body, log a period.

### Phase 3 — Sensor detector + blender (3 недели)
- Temperature CUSUM + Marshall threshold
- HRV/RHR luteal signal
- Bayesian confidence fusion
- Sensor predictor + α-blender
- Outlier rejection
- Daily batch job (TZ-aware)

**Exit criteria:** synthetic 60-day fixtures produce predictions matching expected ground truth ≥80%.

### Phase 4 — 3 остальных таба + push notifications (2 недели)
- Calendar/Insights/History tabs
- 5 push categories + APNs scheduling
- Watch complication
- Widget

**Exit criteria:** all 4 tabs functional, all push types fire correctly in dev environment.

### Phase 5 — Edge cases + analytics + tuning (2 недели)
- Lifecycle state machine (cold_start → active → anovulatory → contraception)
- PCOS escape hatch UI
- Hormonal contraception branch
- AI narrator integration (cycle_narrative endpoint)
- Analytics events
- Sensitivity module integration
- Snapshot tests

**Exit criteria:** all scenario flows tested, analytics fired, no crashes in 1-week QA.

### Phase 6 — Beta + LH validation (4 недели — параллельно с Phase 5)
- Recruit 50 beta users via TestFlight
- Minimum 2 cycles nightly wear
- Optional academic partnership for LH cohort
- Iterate thresholds based on data

**Exit criteria:** retrospective ovulation ≥80%, beta NPS ≥40, no critical bugs.

### Phase 7 — Public launch (1 неделя)
- Feature flag flip (server-side)
- Gradual rollout: 10% → 50% → 100% over 5 days
- Monitor: error rate, push delivery rate, retention
- Press release with conservative claims (not 100%, ~80% ovulation accuracy)
- Marketing copy passed legal review

**Total: ~16 weeks (4 months) from kickoff to public launch.**

---

## Critical files to be modified/created

### Backend (`wvi-api-rust/`)
**New:**
- `src/cycle/` — entire module (~25 files, ~3500 LOC)
- `migrations/018_cycle_tracking.sql`
- `tests/cycle_test.rs`
- `docs/dpia/2026-04-cycle-tracking.md`

**Modified:**
- `src/main.rs` — register cycle routes (~lines 239–474), spawn cycle daily batch task (~line 167-style)
- `src/ai/prompt_rules.rs` — extend Thermoregulation skill (line 34) с cycle-specific guidance
- `src/ai/handlers.rs` — add `AiEndpointKind::CycleNarrative`
- `src/ai/precompute.rs` — prewarm cycle narrative для active female users
- `src/sensitivity/handlers.rs` — add cycle-correlated signals в insights
- `src/users/` — add `country` column для RU exclusion (если ещё нет)

### iOS (`WVIHealth/`)
**New (~25 files):**
- `Features/Onboarding/Screens/{GenderSelection,CycleTrackingWelcome,CycleContraception,CyclePeriodAnchor}View.swift` (4 файла)
- `Features/Cycle/CycleHomeView.swift`
- `Features/Cycle/Tabs/{CycleToday,CycleCalendar,CycleInsights,CycleHistory}Tab.swift` (4 файла)
- `Features/Cycle/Sheets/{PeriodLog,SymptomLog}Sheet.swift` (2 файла)
- `Features/Cycle/CycleViewModel.swift`
- `Features/Cycle/CycleAPIClient.swift`
- `Features/Cycle/CycleSummary.swift` (model)
- `Features/Body/Cards/{CycleSummaryCard,CycleEmptyCard}.swift` (2 файла)
- `WVIHealthWatch/CycleComplicationProvider.swift`
- `WVIHealthWidget/CycleWidget.swift`
- Snapshot tests (3 файла)

**Modified:**
- `Core/Design/Theme.swift` — add `WVIColor.cycle = Color(hex: 0xC026D3)` (line ~59)
- `Core/Localization.swift` — +60 entries
- `Core/Logging/AnalyticsEvent.swift` — +14 cycle events
- `Core/Notifications/PushNotificationManager.swift` — 5 new categories, deeplink routing
- `Core/Navigation/NavigationCoordinator.swift` — add `AutoscrollTarget.cycleCard`
- `Features/Onboarding/OnboardingState.swift` — 4 new stages
- `Features/Onboarding/OnboardingCoordinator.swift` — 4 new transition methods + state
- `Features/Onboarding/RootRouterView.swift` — 4 new switch cases
- `Features/Body/BodyScreen.swift` — add cycleSummary state + onAppear load
- `Features/Body/BodyScreen+Sections.swift` — add cycleSection ViewBuilder
- `Features/Settings/SettingsView.swift` — add Cycle Tracking section
- `App/WVIHealthApp.swift` — register cycle widget, complication descriptors
- `App/ContentView.swift` — handle `wellex://cycle/today` deeplink

**Reused without modification:**
- `Core/Design/Components/{RingProgress,GlassCard,WellexButton,BackButton,PeriodSelector,SkeletonModifier,AIInsightCard}.swift`
- `Core/Design/Components/SharedComponents.swift` (ScanRings, GlowCard)
- `Core/Auth/SecureStorage.swift`
- `Features/Onboarding/Screens/PersonalizeView.swift` (template)
- `Features/Details/MetricDetailScreen.swift` (template для inspiration, не реюз напрямую)

---

## Telemetry & Monitoring

### Prometheus metrics (Rust)
- `cycle_onboarding_started_total{contraception_method}`
- `cycle_onboarding_completed_total{contraception_method}`
- `cycle_predictions_generated_total{method}` (calendar/sensor/blended)
- `cycle_detection_confidence_histogram`
- `cycle_anovulatory_detected_total`
- `cycle_push_delivered_total{category}`
- `cycle_push_failed_total{category, reason}`
- `cycle_export_requests_total`
- `cycle_delete_requests_total`
- `cycle_api_latency_histogram{endpoint}`
- `cycle_active_users_total{lifecycle_state}`

### Sentry / OTel traces
- Каждый cycle endpoint sampled at 5% (default head sampling)
- Detector batch job traced как single span per user per day
- Errors при detection (insufficient data, contradictions) — captured as warnings
- Push send failures (404/410 on token) — captured

### Health dashboards
- Cycle accuracy dashboard: rolling 30-day ovulation detection rate vs LH cohort (когда появится)
- Push engagement: delivery → tap rate per category
- Onboarding funnel: gender_selection → cycle_welcome → consent → contraception → anchor → completed
- Lifecycle distribution: % users in each state

### Alerts
- Detector batch job failure → PagerDuty
- Push delivery rate <80% → Slack alert
- New users onboarding rate dropping >30% week-over-week → Slack
- DB query latency on cycle_phases >500ms p95 → Slack

---

## Risks & Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | JCV8 skin temp noise floor unknown — может быть выше 0.24°C | Medium | High | Pre-launch: empirical SD measurement on 10-user cohort over 30 nights. Adjust threshold up if needed. |
| 2 | Bracelet снимается ночью — нет данных | High | Medium | Coverage badge на Today tab: "Wear nightly for accurate predictions"; lower confidence at low coverage; calendar fallback always works |
| 3 | Calendar и sensor расходятся в первые 30 дней | Medium | Low | Явное "calibrating" сообщение, не показываем conflicting predictions, sensor wins after 30 days |
| 4 | Gender в SecureStorage не truthful (gender=male, но имеет цикл) | Low | Low | Settings → "Manually enable cycle tracking" override (Phase 4 add) |
| 5 | False positive ovulation от illness/fever spike | Medium | Medium | RHR >110 outlier reject; require 3 consecutive nights; user can override via period log |
| 6 | PMS push раздражает 30%+ пользовательниц | Medium | Medium | Default opt-in BUT при первом срабатывании: "Helpful? [Yes/No/Less often]"; auto-tune frequency |
| 7 | RU 152-ФЗ violation если EU shard принимает RU users | Low | Critical | Geo-fence на gateway level via `users.country` или device locale; "Coming soon" banner |
| 8 | GDPR fine для health data leak | Low | Critical | Encryption at rest already on Postgres, encryption in transit (TLS 1.3); access audit logs; quarterly security review |
| 9 | False contraceptive claim → FDA enforcement | Low | Critical | Никогда не использовать слово "contraceptive" или "birth control" в копи; legal review копи перед launch |
| 10 | Beta cohort показывает <70% ovulation accuracy | Medium | High | Tune CUSUM thresholds; consider postponing public launch; possibly restrict to "regular cyclers only" in v1 marketing |
| 11 | Localization задерживает launch | Low | Low | Запустить EN-only initially, остальные локали — fast follow |
| 12 | RU users пытаются использовать через VPN | Medium | Medium | Geo-fence is best-effort; explicit ToS section: "by enabling, you confirm you are not a Russian resident"; legal cover |

---

## Out of scope (Phase 2+)

- **Pregnancy mode** — V8 SDK имеет `setPregnancyInfo_V8` placeholder, но включение требует отдельной compliance-работы (special category data + minor risk)
- **Medical insights** ("ваш цикл нерегулярный → к врачу" в АКТИВНОМ pushe) — требует CE/FDA classification или explicit medical disclaimer wall
- **Apple Watch full app** для Cycle (только complication для MVP)
- **Family/partner sharing**
- **Full ML модель** (LSTM на тайм-сериях) — требует 5000+ размеченных циклов, 12+ месяцев работы
- **Natural Cycles API integration** (для contraceptive claim) — потенциально Phase 3 как партнёрство
- **Kegel/perineal training reminders**
- **PCOS deep-dive features** (дополнительные метрики)

---

## Verification (end-to-end)

### Backend smoke
```bash
cd /Users/alexander/Code/wvi-api-rust
sqlx migrate run
cargo test cycle::                    # all unit tests pass
cargo run                             # API on :8091

# Test flows:
TOKEN="dev-token"
curl -H "Authorization: Bearer $TOKEN" -X POST localhost:8091/api/v1/cycle/onboarding \
  -d '{"contraception_method":"none","last_period_date":"2026-04-13","avg_cycle_length_days":28,"consent_special_category":true}' \
  -H "Content-Type: application/json"

curl -H "Authorization: Bearer $TOKEN" localhost:8091/api/v1/cycle/state
# Expect: phase, day, predictions с alpha=0 (calendar-only)

curl -H "Authorization: Bearer $TOKEN" -X POST localhost:8091/api/v1/cycle/period-log \
  -d '{"date":"2026-04-13","flow_intensity":"medium","is_first_day":true}' \
  -H "Content-Type: application/json"

# Hormonal contraception flow
curl -H "Authorization: Bearer $TOKEN" -X PATCH localhost:8091/api/v1/cycle/profile \
  -d '{"contraception_method":"pill"}'
curl -H "Authorization: Bearer $TOKEN" localhost:8091/api/v1/cycle/state
# Expect: ovulation hidden, only period prediction

# Gender gating: change user gender to male, expect 404 on all cycle endpoints
```

### iOS smoke
```bash
cd /Users/alexander/Code/WVIHealth
xcodebuild -scheme WVIHealth -destination 'platform=iOS Simulator,name=iPhone 16' build

# Manual smoke:
# 1. Reset onboarding (long-press logo на splash)
# 2. Walk: GenderSelection → выбрать Female
# 3. CycleTrackingWelcome: toggle consent, Continue
# 4. CycleContraception: select "None or non-hormonal"
# 5. CyclePeriodAnchor: pick 14 days ago, Continue
# 6. Pass remaining onboarding (Personalize, etc.)
# 7. On Body screen: Cycle card visible с фазой + day
# 8. Tap → CycleHomeView opens с Today tab
# 9. Tap "Log period" → sheet → save → calendar tab shows logged day
# 10. Tap "Log symptom" → save → insights tab shows pattern
# 11. Settings → Cycle Tracking section visible
# 12. Disable master toggle → Cycle card disappears, data preserved
# 13. Re-enable → card returns
# 14. Repeat for gender=Male — Cycle section nowhere visible
# 15. Test push receipt: trigger via dev tool, deeplink opens Today tab
```

### End-to-end (after Phase 3)
- Mock 60 nights skin temp + HRV + RHR в DB через тест-скрипт `cargo run --bin seed_cycle_data -- --user-id X --pattern regular`
- Trigger `POST /api/v1/cycle/admin/recompute`
- Verify cycle с ovulation_date, ovulation_confidence ≥0.7, method='sensor'
- Verify predictions: blender_alpha=1.0, predicted_period в пределах ±2 дня от actual (по seed)
- Daily batch: запустить `cargo run --bin run_cycle_daily_batch` → проверить cycle_push_log записи

### Beta validation (Phase 6)
- 50 users × 2 cycles minimum
- Compare detected ovulation_date with self-reported "felt ovulation symptoms"
- Compute accuracy ±2 days target ≥80%
- Survey: NPS ≥40, "would recommend" ≥70%
- No crashes in production logs related to cycle for 2 consecutive weeks

---

## Additional details (iterations 2-10)

### Algorithm versioning

Каждое детектирование (ovulation_events, cycle_predictions) хранит `algorithm_version` (e.g. `'v1.0_marshall_cusum'`). При апгрейде алгоритма:
- Новые detections → новая версия
- Старые НЕ перерасчитываются автоматически (UX disruption)
- Admin endpoint `POST /cycle/admin/recompute?version=v1.1` для batch reprocessing
- Сохраняется audit trail: какой алгоритм поставил какой confidence на какой день

Версии в плане:
- `v1.0_marshall_cusum` — MVP (rule-based)
- `v1.1_personal_baselines` — после 3+ циклов на user, переход на personal SD/mean (Phase 6 tuning)
- `v2.0_population_ml` — после 10k+ logged cycles, population gradient boosting
- `v3.0_ml_lh_validated` — после LH-validated cohort

### Apple HealthKit integration

**Read:** Phase 1 — none (мы используем JCV8 напрямую через V8 SDK, не зависим от HealthKit). Hormonal contraception data из HealthKit → можно опционально подсосать в onboarding для удобства, но primary source — наш onboarding screen.

**Write back:** Phase 5 — write-back cycle data в HKCategoryTypeIdentifier.menstrualFlow + HKCategoryTypeIdentifier.ovulationTestResult (если соответствующая permission granted). Это даст пользовательнице консистентность с iOS Health app. Permission scope строго ограниченный, написать обоснование в Privacy Manifest.

**Important:** не обязательно для launch, но добавляет network effect (Apple Health users увидят cycle data синхронизированной). Phase 5 add.

### Failure modes & resilience

| Failure | Impact | Behavior |
|---|---|---|
| `/api/v1/cycle/state` returns 500 | iOS shows last-known cached state from `BiometricCache.cycle_*` keys; banner "Reconnecting..." | Retry exponential backoff |
| Daily batch detector job fails | No new predictions for that day; previous predictions still valid (валidnost via `valid_until` field) | Pager alert; auto-retry next hour |
| AI narrator service unavailable | Today tab без AI insight, остальное работает | Fallback to template-based copy |
| Postgres replica lag | Stale predictions (max ~30s) | Acceptable; predictions change daily, не по секундам |
| Push delivery fails (APNs 410) | Token removed from push_tokens, user не получает push | Logged; user re-registers token on next app open |
| User changes timezone (travel) | Daily batch может сработать дважды или ноль раз | Log fire с UTC start-of-local-day, ON CONFLICT DO NOTHING; max 1 fire per local day |
| Bracelet not worn for 2 weeks | Sensor predictions become stale | Lower confidence на UI; revert to calendar after 7 days no data |

### Push notification copy (with required disclaimers)

Каждый push ≤ 110 chars body (APNs limit ≈ 200 with safe margin).

**CYCLE_FERTILE:**
- Title: "Your cycle window"
- Body: "Estimated ovulation around tomorrow. Tap for details."
- Footer: "General wellness — not medical advice"

**CYCLE_PERIOD_COMING:**
- Title: "Period in 2 days"
- Body: "Estimated start: Apr 16. Plan accordingly."
- Footer: "General wellness — not medical advice"

**CYCLE_DELAY:**
- Title: "Period running late"
- Body: "Your period was estimated for Apr 14. Log when it starts."
- Footer: "Not a pregnancy test. Consult a clinician if needed."

**CYCLE_PMS:**
- Title: "PMS pattern detected"
- Body: "Based on prior cycles, you may feel sensitive in the next few days."
- Footer: "General wellness — not medical advice"

**CYCLE_INSIGHT (opt-in):**
- Title: "Cycle complete"
- Body: "Your cycle was 28 days. Last 3 cycles avg: 28.3 days."
- Footer: "Insights generated from your data"

Локализация: каждая строка через `L.push_*` ключи, переводится на 6 языков.

### Contraception changes mid-flight

Если пользовательница изменяет `contraception_method` в Settings:
- **none → hormonal**: ovulation predictions немедленно скрываются. Период предупреждения: "Hormonal contraception affects cycle signals. Predictions may be off for the first 1-2 cycles."
- **hormonal → none**: показываем banner "Resuming detection. May take 1-2 cycles to recalibrate after stopping hormonal contraception."
- **hormonal → hormonal (different)**: то же самое — recalibration phase
- В DB: всегда новая row в `cycle_consent_log` с `consent_type='contraception_change'`, новая запись в `cycle_profiles.updated_at`

### Pregnancy toggle (Phase 2 only — disabled in MVP)

В MVP — toggle present but disabled with "Coming in next update" copy. В Phase 2:
- Toggle "I'm pregnant" в Settings
- Если true: cycle predictions останавливаются, показываем weeks tracker (28-week countdown), pregnancy-specific health metrics
- Сохраняем все cycle data для post-pregnancy resumption
- Дополнительные disclaimer'ы про prenatal care / consult OB-GYN

### A/B testing setup

Для tuning после launch (Phase 6+):

| Experiment | Variants | Metric | Sample size |
|---|---|---|---|
| Confidence threshold для push | 0.5 / 0.6 / 0.7 | Push tap rate | 500 users × 30 days |
| Calendar weight в blender | α floor 0 / 0.1 / 0.2 | Period prediction accuracy | 1000 users × 60 days |
| PMS push timing | -3 / -4 / -5 days | "Was helpful" survey rate | 300 users × 3 cycles |
| Onboarding contraception question wording | A/B | Onboarding completion % | 500 users |

Implementation: env-based feature flags + `users.experiment_groups` JSONB column.

### Consent UX exact wording

**On CycleTrackingWelcome screen:**

> ### Cycle understanding built into your body
>
> Your bracelet's sensors detect your menstrual cycle, ovulation, and period delays automatically — using nightly skin temperature, heart rate variability, and resting heart rate.
>
> **Your data:**
> - Stored encrypted at rest and in transit
> - Used only to power your cycle insights
> - Never shared with third parties for advertising or sold
> - You can export it or delete it anytime in Settings
>
> **What this is, and is not:**
> - **Is**: a wellness feature to help you understand patterns in your cycle
> - **Is not**: a medical device, a diagnostic tool, or a contraceptive method
>
> Cycle data is special-category health data under GDPR Article 9. We process it only with your explicit consent.
>
> [ ] I consent to processing of my cycle health data for the purposes above (required)
>
> [Continue]
> [Skip cycle tracking]

Toggle required, "Continue" disabled until checked. "Skip" — fully opts out, cycle features remain hidden permanently (можно включить позже в Settings).

### DPIA outline (для compliance team)

Документ `/Users/alexander/Code/wvi-api-rust/docs/dpia/2026-04-cycle-tracking.md` структура:

1. **Description of processing**: что собираем, откуда, кому передаём
2. **Necessity and proportionality**: почему это необходимо для feature, почему минимизируем
3. **Risks to data subjects**: re-identification, leak, misuse
4. **Mitigations**: encryption, access logs, retention policies, consent
5. **Lawful basis**: Art 9(2)(a) explicit consent
6. **Data subject rights flows**: how Art 15/16/17/20 are honored
7. **Security measures**: pgcrypto, TLS 1.3, rate limiting, access audit
8. **Retention**: cycle_* data deleted on user request OR 7 years inactive (per FTC Health Breach Rule)
9. **DPO contact**: alex@crossfi.org
10. **Sign-off**: legal review, last reviewed date

### Account deletion behavior

При удалении user account (existing endpoint):
- ON DELETE CASCADE автоматически удаляет cycle_*, period_logs, symptom_logs, cycle_consent_log, cycle_signals_nightly
- audit_log retains user_id reference (anonymized after 90 days per existing policy)
- 30-day soft-delete window (consistent с existing policy): данные восстановимы для полной 30 дней

### New bracelet handling

Если пользовательница меняет JCV8 на новый (через WellexDeviceManager):
- Skin temp baseline reset (новый сенсор может иметь systematic offset)
- 7-day re-calibration period: predictions используют только calendar predictor, sensor data игнорируется
- В Today tab: "Recalibrating after device change"
- После 7 дней — новый baseline установлен, sensor predictor возобновляется

### Battery / data implications

JCV8 уже всегда включает skin temp + HRV ночью (для общих metric'ов). Cycle module не добавляет нового sensor load. Backend cost:
- Daily batch: ~3 sec compute per user, ~10MB DB read, ~5MB write per user per day
- Storage: ~500 KB per user per year (cycle_signals_nightly dominates)
- При 100k active female users: 50 GB cycle data per year, $5/mo Postgres storage (S3-compatible) — negligible

### Documentation

Обновить:
- `/Users/alexander/Code/WVIHealth/ARCHITECTURE.md` — добавить cycle module section
- `/Users/alexander/Code/WVIHealth/docs/cycle/onboarding.md` — flow doc с screenshots для design review
- `/Users/alexander/Code/wvi-api-rust/docs/cycle/algorithm.md` — алгоритм с математикой и citations
- App Store reviewer notes — paragraph про general-wellness positioning

### Iteration log of decisions

10-итерационный self-review захвачен в этом плане. Конкретные итерации:

1. **Iteration 1 (initial):** scope MVP B, cold-start B, algorithm B, UI B
2. **Iteration 2 (research-informed):** добавлен hormonal contraception branch, PCOS escape hatch, exact thresholds (0.20°C / 3 nights / 6-day baseline)
3. **Iteration 3 (compliance):** добавлены GDPR DPIA, 152-ФЗ Russia exclusion, FDA general-wellness positioning, exact disclaimer text
4. **Iteration 4 (UX rigor):** consent toggle wording, push copy with disclaimers, anovulatory escape hatch UX
5. **Iteration 5 (architecture):** Kafka subscriber pattern, AI prompt extension, sensitivity module integration, lifecycle state machine
6. **Iteration 6 (i18n + a11y):** 60+ localized strings, 6 languages, VoiceOver labels, Dynamic Type support, Reduce Motion fallbacks
7. **Iteration 7 (extension surfaces):** Watch complication, Widget, Apple HealthKit write-back (Phase 5), Analytics events
8. **Iteration 8 (testing):** synthetic fixtures (5 patterns), beta cohort plan, LH validation partnership, snapshot tests, A/B testing setup
9. **Iteration 9 (operations):** Prometheus metrics, Sentry/OTel tracing, alert thresholds, rollout %, feature flags
10. **Iteration 10 (resilience):** algorithm versioning, contraception migration, new bracelet recalibration, account deletion, failure modes

---

## References (research sources)

- Apple Hum Reprod 2025 (Symul/Apple, n=260, 889 cycles): wrist temperature for retrospective ovulation. https://academic.oup.com/humrep/article/40/3/469/7989515
- Oura validation, JMIR 2025 (n=964, 1,155 ovulatory cycles): 96.4% ovulation detection, MAE 1.26 days. https://pmc.ncbi.nlm.nih.gov/articles/PMC11829181/
- WHOOP, npj Digital Medicine 2024 (n=11,590, 45,811 cycles): RHR +2.73 BPM, HRV -4.65 ms in luteal. https://www.nature.com/articles/s41746-024-01394-0
- Maijala et al. 2019 (Oura, n=22, 99 menstruations): finger ΔT 0.30°C ± 0.12. https://pmc.ncbi.nlm.nih.gov/articles/PMC6883568/
- Zhu et al. 2021 (Ava, n=57, 193 cycles): WST 0.50°C luteal shift, sensitivity 0.62. https://pmc.ncbi.nlm.nih.gov/articles/PMC8238491/
- Schmalenberger 2020: HRV and progesterone correlation. https://pmc.ncbi.nlm.nih.gov/articles/PMC7141121/
- WHO PCOS fact sheet: 10–13% prevalence, 80% of female anovulatory infertility. https://www.who.int/news-room/fact-sheets/detail/polycystic-ovary-syndrome
- Symul/Bull Hum Reprod Open 2020: only 12.4% of women have true 28-day cycle, 52% have ≥5d variability. https://academic.oup.com/hropen/article/2020/2/hoaa011/5820371
- FDA General Wellness Policy. https://www.fda.gov/regulatory-information/search-fda-guidance-documents/general-wellness-policy-low-risk-devices
- Natural Cycles De Novo DEN170052 (FDA-cleared algorithmic contraceptive). https://www.accessdata.fda.gov/cdrh_docs/reviews/DEN170052.pdf
- ICO special category data guidance. https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/lawful-basis/a-guide-to-lawful-basis/special-category-data/
- Russia 152-ФЗ overview. https://securiti.ai/russian-federal-law-no-152-fz/
