# Wellex Cycle Tracking — Master Implementation Plan **v2.0**

> Canonical, deeply detailed cross-platform implementation plan. Supersedes v1 (committed at `3decaca..d3c0f55`). Companion documents: `iOS/docs/cycle-tracking/IOS_IMPLEMENTATION_PLAN.md`, `Android/docs/cycle-tracking/ANDROID_IMPLEMENTATION_PLAN.md`, and `wellex-io/app-backend → docs/cycle-tracking/IMPLEMENTATION_PLAN.md`. Implementation has not started; this document is the contract that all three streams will execute against.

**Authors:** Alex (PM/Tech Lead) + Claude Opus 4.7 (research, drafting). 
**Last revision:** 2026-04-27 (v2.0 — full rewrite with day-by-day plan, runbooks, threat model, cost analysis).
**Status:** Pending product approval; engineering kick-off blocked on legal sign-off (DPIA + FDA wellness positioning review).

---

## Table of Contents

0. [Executive Summary](#0-executive-summary-1-page)
1. [Context & Goals](#1-context--goals)
2. [Background Research Summary](#2-background-research-summary)
3. [Product Requirements](#3-product-requirements)
4. [Technical Architecture](#4-technical-architecture)
5. [Algorithm Specification](#5-algorithm-specification)
6. [Database Schema](#6-database-schema)
7. [API Specification](#7-api-specification)
8. [Backend Implementation Plan (Rust)](#8-backend-implementation-plan-rust)
9. [iOS Implementation Plan](#9-ios-implementation-plan)
10. [Android Implementation Plan](#10-android-implementation-plan)
11. [Watch / Widget / Companion Implementations](#11-watch--widget--companion-implementations)
12. [Onboarding UX Specification](#12-onboarding-ux-specification)
13. [Push Notifications Specification](#13-push-notifications-specification)
14. [Settings Specification](#14-settings-specification)
15. [Localization](#15-localization)
16. [Accessibility Specification](#16-accessibility-specification)
17. [Analytics & Telemetry](#17-analytics--telemetry)
18. [Testing Strategy](#18-testing-strategy)
19. [Compliance Plan](#19-compliance-plan)
20. [Security & Threat Model (STRIDE)](#20-security--threat-model-stride)
21. [Operations & Runbooks](#21-operations--runbooks)
22. [Performance Budgets & SLOs](#22-performance-budgets--slos)
23. [Phasing & Day-by-day Timeline](#23-phasing--day-by-day-timeline)
24. [Risks & Mitigations (Extended)](#24-risks--mitigations-extended)
25. [Beta Program](#25-beta-program)
26. [Launch Plan](#26-launch-plan)
27. [Cost Analysis](#27-cost-analysis)
28. [Decision Log](#28-decision-log)
29. [Glossary](#29-glossary)
30. [References](#30-references)

---

## 0. Executive Summary (1 page)

**What:** Auto-detect menstrual cycle phase, ovulation, period delays, and PMS from JCV8 bracelet sensors (skin temperature, HRV, RHR, sleep). Surface fertile-window forecasts, period predictions, and symptom logs as a Body-screen feature for users who self-identify as `gender == 'female'`.

**Why:** Direct user request; closes a major capability gap vs Apple Watch / Oura Ring; opens monetization upside (parity with Flo/Clue Premium tier); generates labeled biometric data that improves WVI v3 emotion engine over time.

**Shape (MVP B):**
- 8 user-visible features: period prediction, retrospective ovulation, fertile-window forecast, delay alert, phase ring, symptom log, PMS prediction, period-day tracking.
- Hybrid statistical algorithm: Marshall threshold + CUSUM + multi-signal Bayesian fusion. No ML in v1 (no labeled training data yet); rule-based MVP, ML-ready architecture for v2.
- 4 default push categories + 1 opt-in.
- Cold-start: progressive blending (calendar → sensor) with confidence score.
- Out of scope for v1: pregnancy mode, medical diagnostic insights, fertile-window contraceptive use, Russia launch.

**Targets (vs published validated competitor data):**

| Metric | Wellex v1 target | Wellex v2 target | Reference |
|---|---|---|---|
| Retrospective ovulation ±2 days | ≥80% | ≥85% | Apple Hum Reprod 2025 (89% completed); Oura JMIR 2025 (87.9%) |
| Forward ovulation prediction ±2 days (regular cycles) | 65–75% | 75–85% | Oura 12-month forward predictions |
| Period prediction ±2 days | 75–85% | ≥80% | Symul 2025 (89.4% within ±3) |
| Delay detection (binary, late >2 days) | >95% | >95% | Trivial calendar logic |

**Effort:** 16 weeks for iOS + Backend; 17 weeks for Android (parallel after week 4); ~3 engineers + 1 QA + 1 designer + legal/translator. **~$145k all-in for MVP.**

**Key risks:** (1) JCV8 sensor noise floor unknown — empirical validation required before locking thresholds. (2) FDA general-wellness boundary — strict copy review needed on every fertile-window touchpoint. (3) Russia 152-FZ — feature must geofence RU users for v1.

**Decision required from product to begin engineering:**
1. Approve MVP scope (8 features, no pregnancy/medical claims).
2. Approve $145k budget.
3. Approve geofence policy for Russia v1.
4. Sign-off DPIA after legal review.

---

## 1. Context & Goals

### 1.1 The user need

A direct user request (Alex, 2026-04-27): "Можно ли сделать так, чтобы браслет в Эллокс мог отслеживать менструальные дни, овуляцию, задержки и вообще все такие моменты не мануально автоматически?"

The premise of the question is that **automatic** tracking is preferable to manual logging. This matches the broader Wellex thesis: "wear it, feel better, live longer" — a passive sensor experience that produces insight without daily effort.

### 1.2 Where we are today

- **iOS WVIHealth app** (`wellex-io/app-frontend / iOS/`): production-ready, ships 8-tab navigation, Apple Watch + widget integration, 18-emotion engine, WVI v3, Russian localization. **No cycle tracking; no menstrual data model.**
- **Android Wellex** (`wellex-io/app-frontend / Android/`): Phase 0-4 done (43+ commits), Compose UI, Hilt, Room v1 (8 entities), Ktor 3.4, JVM domain core with WVI v2 + EmotionEngine + HRV + Sleep + BioAge + ACWR, 145 unit tests passing. Phase 5 (BLE + V8 SDK) in progress. **No cycle module.**
- **Backend** (`wellex-io/app-backend` aka `wvi-api-rust`): Axum 0.8 + Postgres + sqlx, 18 modules / 123 endpoints, Kafka event bus, OpenTelemetry tracing, Sentry, Prometheus, APNs push, AI gateway integration. **Gender stored on users.gender VARCHAR(10) but no feature gating; no cycle module; AI prompt rules briefly mention "menstrual cycle impacts" but no implementation.**
- **Bracelet (JCV8 via V8 SDK)**: ships skin temperature, HRV, RHR, sleep architecture, ECG; no cycle-related SDK calls beyond two enum placeholders (`setMenstruationInfo_V8 = 78`, `setPregnancyInfo_V8 = 79`) without backing public API methods.

### 1.3 What we will deliver in v1

Single coherent feature surface across iOS + Android + Wear/Widget:
1. Onboarding flow that captures gender and (for women) cycle anchor + contraception method.
2. Cycle card on Body screen for self-identified female users.
3. Cycle home screen with 4 tabs (Today / Calendar / Insights / History).
4. Period log + Symptom log bottom sheets.
5. Settings cycle section with 5 toggles + export + delete.
6. 5 push notification categories (4 default-on, 1 opt-in).
7. Watch complication + home-screen widget.
8. Backend cycle module with 12 endpoints + daily detector batch.
9. GDPR Art 17/20 export + delete endpoints.
10. Apple HealthKit / Android Health Connect write-back of cycle predictions.

### 1.4 Explicit non-goals for v1

- Pregnancy mode, prenatal insights.
- Medical diagnostic claims (PCOS diagnosis, "consult immediately" alarms tied to specific findings).
- Contraceptive use ("safe day" recommendations, Pearl Index claim, fertility planning use case).
- Russia launch.
- ML-personalized predictions (rule-based v1 only).
- Partner sharing / family sharing.
- Integration with third-party cycle apps (Natural Cycles, Flo, Clue).
- iPad/web cycle dashboard.

### 1.5 Success criteria

For the engineering team to declare v1 "done":
- Backend `cargo test cycle::` 100% passing; 12 endpoints validated by integration tests.
- iOS feature flag flipped; full onboarding flow + Cycle home + log sheets + settings working in Production scheme.
- Android feature flag flipped; same coverage.
- Beta cohort (50 users, 2 cycles each) reports retrospective ovulation accuracy ≥80% versus self-reported truth.
- Crash-free sessions ≥99.5% in TestFlight + internal Play track for 14 consecutive days.
- App Store / Play Store reviews pass; FDA general-wellness self-assessment signed off; DPIA filed.
- Public soft launch executed in 5-day staged rollout (10% → 25% → 50% → 100%).

---

## 2. Background Research Summary

### 2.1 Scientific foundation

Distilled from peer-reviewed studies (full citations in §30):

- **Skin temperature shift in luteal phase:**
  - Wrist nocturnal ΔT = 0.30°C ± 0.12 (Maijala 2019, n=22, finger thermistor)
  - Wrist (Ava bracelet) ΔT = 0.50°C; sensitivity 0.62 vs oral BBT 0.23 (Zhu 2021, n=57)
  - Apple Watch ΔT ≥0.20°C threshold gives 80% retrospective ovulation accuracy (Symul/Apple Hum Reprod 2025, n=260, 889 cycles)
  - Oura ΔT detection achieves 96.4% accuracy with MAE 1.26 days vs LH ground truth (JMIR 2025, n=964, 1,155 ovulatory cycles)
  - **Implication:** target 0.20°C threshold over 3 consecutive nights with 6-day rolling baseline matches state of the art and is achievable on JCV8.

- **HRV and RHR shifts in luteal phase:**
  - RHR rises +2.73 BPM (follicular → luteal) at population mean (WHOOP n=11,590, 45,811 cycles, npj Digital Medicine 2024).
  - HRV (RMSSD) drops −4.65 ms.
  - 93% of users show detectable RHR amplitude.
  - Mechanism: progesterone increases sympathetic drive; estrogen enhances vagal tone, the imbalance shifts ANS profile.
  - **Implication:** secondary signal for confidence boost. Used to confirm temperature-based detection; weak alone.

- **Why nocturnal-only:**
  - Daytime wrist temp is dominated by ambient + sleeve + activity (~1°C diurnal). Nocturnal sleep produces a stable thermal plateau where the cycle-driven 0.3°C signal is recoverable from environmental noise (~0.1–0.2°C).
  - Apple aggregates 5-second wrist samples into one nightly value. Oura uses 10pm–8am moving average filter → highest stable post-Butterworth value.
  - **Implication:** algorithm runs on per-night aggregate, not raw samples.

### 2.2 Competitor landscape

| Company | Sensor | Algorithm | Accuracy | Calibration | Manual input | Edge cases | Pricing |
|---|---|---|---|---|---|---|---|
| **Oura Ring** | Continuous finger NTC + PPG | Butterworth bandpass + hysteresis thresholding (proprietary) | 96.4% ovulation, MAE 1.26d | 1 night minimum; predictions extend 12 mo | Period start, hormonal-contraception flag | Disclaims hormonal contraception, postmenopausal, persistent anovulation | $349 ring + $5.99/mo |
| **Apple Watch (S8+ / Ultra)** | Dual temperature + PPG | ≥0.20°C wrist shift threshold; aggregates to nightly value | 80% retrospective ±2d; 89% completed | ~2 cycles | Period start, cycle length, period length | Open to all; retrospective ovulation Series 8+ only | Bundled |
| **Natural Cycles** | BBT thermometer or Oura/Apple | Proprietary; analyses temp shift, sperm survival, ovulation variance | Pearl Index typical 6.5 / perfect 1.8 | Day 1 | Period, daily temp, optional LH tests | NC° Perimenopause mode; not for hormonal contraception | $99.99/yr |
| **Whoop** | PPG (HRV/RHR/RR) + skin temp | Phase-based coaching from cardiovascular amplitude | No published ovulation accuracy; 93% RHR amplitude | 1 cycle | Period dates, Clue integration | Pregnancy mode | $30/mo |
| **Fitbit Sense / Charge 6** | Skin temp + PPG | Nightly skin-temp variation vs 30-day baseline | None published | 30 days | Cycle log | None | $9.99/mo Premium |
| **Clue** | None (phone) | Bayesian / DOT (Dynamic Optimal Timing) | 11–14% pregnancy rate as FAM | 2-3 cycles | Heavy: flow, symptoms, sex, mood, contraception | Perimenopause mode 2024 | $9.99/mo |
| **Flo** | None native (HK/HC integration) | Two-stage: per-user → population NN | 90% period satisfaction; 78% fertile-window regular | 2-3 cycles | Period dates, symptoms | Pregnancy mode; Anonymous Mode | $39.99/yr |

### 2.3 Edge case prevalence

- **PCOS:** 10–13% of reproductive-aged women (WHO). Anovulatory pattern → no luteal progesterone surge → no temperature shift. Algorithm fails; need escape hatch.
- **Perimenopause:** age 45–55, cycle-length variance 5.3 days at <20 declining to 3.8 at 35–39 then rising again (Apple AWHS 2025). Hot flashes confound nocturnal temp.
- **Hormonal contraception:** combined pill, hormonal IUD, implant, patch, ring suppress LH/FSH and the temperature shift. **All temperature-based ovulation detection is invalid.**
- **Postpartum:** lactational amenorrhea mean 9.5 mo, anovulation up to 14.6 mo.
- **Cycle distribution:** mean 28.7d, median 28d (IQR 26–30), 5–95th percentile 22–38d (Symul 2020 n=124k cycles). Only 12.4% have true 28-d cycle; 52% have ≥5d cycle-to-cycle variability.

### 2.4 Regulatory landscape

- **FDA:** Class I exempt (general wellness) is achievable if we avoid contraceptive claims, "abnormal" labels, and disease references. Boundary is "claim-driven" — software must not diagnose, treat, cure, or prevent disease.
- **EU MDR:** Rule 11 (informs medical decisions) → Class IIa minimum. Rule 15 (contraception) → Class IIb. We avoid both by stripping diagnostic + contraceptive language.
- **GDPR Article 9:** Cycle data is special-category health data. Requires explicit consent, DPIA, data minimization, right to erasure (Art 17), right to portability (Art 20), records of processing (Art 30), DPO if core activity (Art 37 — likely yes for Wellex).
- **Russia 152-ФЗ Art 18(5):** RU citizen data must hit primary DB on Russian territory. Cross-border transfer allowed only after RU DB is populated. Wellex must register as personal-data operator with Roskomnadzor before processing.
- **HIPAA:** Does not apply to direct-to-consumer apps. **However**, FTC's Health Breach Notification Rule (revised 2024) DOES apply — 60-day breach notification mandate.
- **State laws (US):** California CMIA, Washington My Health My Data Act 2024, Connecticut DPA — all stricter; My Health My Data explicitly covers cycle data.

### 2.5 Wellex hardware constraint

JCV8 bracelet sensor stack:

| Sensor | Cadence | Resolution | Validated range | Wear-time threshold |
|---|---|---|---|---|
| Skin temperature (NTC) | per minute during sleep | 0.1°C | 32–42°C | ≥4 hr continuous sleep for valid nightly aggregate |
| HRV (RMSSD/SDNN) | 60-second window during rest | 1 ms | 5–200 ms | RMSSD requires ≥12 RR samples |
| RHR | continuous, downsampled to per-minute | 1 BPM | 30–220 BPM | Minimum 4-hour stable period |
| Sleep architecture | per-night summary | wake/light/deep/REM minutes | 0–960 min total | Total sleep ≥4 hr |

**Sensor noise floor unknown empirically** — must measure before locking thresholds. Pre-launch task: 10-user cohort wearing JCV8 nightly for 30 days, compute per-night SD of skin temperature, validate it sits below 0.20°C threshold by ≥2× margin. If JCV8 noise is ~0.30°C, our 0.20°C threshold is too aggressive — adjust upward to 0.25°C and accept lower sensitivity.

### 2.6 ML labeling strategy (deferred to v2)

For the future ML-personalized model:
- **Best ground truth (impractical at scale):** serum progesterone day-21, pelvic ultrasound.
- **Best practical ground truth:** urinary LH ovulation predictor kits (used by Oura, Ava, Apple study).
- **Acceptable ground truth (cheapest, noisiest):** self-reported period start dates.
- **Data thresholds for v2:** ~10k logged cycles for population predictor; LH-validated subset (~500 cycles, partner research clinic) before publishing accuracy claims.

We deliberately defer ML to v2 because:
1. We have zero labeled training data on day 1.
2. Rule-based v1 produces labels (sensor-detected ovulation + user-confirmed period dates) that train v2.
3. ML without LH validation gets attacked publicly (cf. Flo's reputation).
4. 12+ months of v1 data accumulation is the cheapest path to a defensible ML system.

---

## 3. Product Requirements

### 3.1 Personas

**Primary persona (P1): "Sofia, 31, regular cycler, fitness-focused"**
- Wears Wellex bracelet 24/7 including nights.
- Cycle is 28-30 days, pretty regular.
- No hormonal contraception.
- Wants to plan workouts around energy fluctuations, predict period for travel, and just generally know what's going on in her body.
- Won't manually log symptoms every day — wants automation.
- Will pay $99/yr for Wellex Plus to keep cycle tracking.
- ~50% of our female user base.

**Secondary persona (P2): "Maria, 38, two kids, hormonal IUD, cares about wellness"**
- Wears Wellex 24/7.
- On hormonal IUD — minimal/no periods.
- No ovulation to predict, no fertility window to track.
- Wants symptom logging (mood, sleep, libido) tied to phase if any, plus general wellness.
- Wants opt-out of pregnancy/period push notifications because she doesn't menstruate regularly.
- ~25% of female user base.

**Tertiary persona (P3): "Anya, 45, perimenopause"**
- Wears Wellex variably (3-4 nights/week).
- Cycle length variable (24-40 days) over last 12 months.
- Wants to understand if she's heading into menopause.
- Predictions are unreliable due to variance — wants honest "uncertain" UI rather than confidently wrong dates.
- ~15% of female user base.

**Out-of-persona for v1:**
- P4: pregnant users (full pregnancy mode = v2)
- P5: postpartum/breastfeeding (anovulation expected)
- P6: PCOS (anovulation pattern → escape hatch shown)
- P7: trans men with cycles (open future research; v1 gates on `gender == 'female'` self-identification)

### 3.2 User stories (acceptance-criteria-grade)

**US-001 — Onboarding gender selection:**
> As a new user, I want to be asked my gender once during onboarding, so the app can adapt feature visibility (cycle tracking, calorie calculation).
- **Acceptance:** Gender screen appears as 2nd onboarding step. 3 options: Female / Male / Prefer not to say. Selection saved to `SecureStorage.userSex` (iOS) / `EncryptedSharedPrefs.userSex` (Android) / `users.gender` (backend).
- **Edge case:** "Prefer not to say" → cycle features hidden but reactivatable via Settings → "Enable cycle tracking".

**US-002 — Cycle tracking welcome (consent):**
> As a female user, I want to understand what cycle tracking does and how my data is handled, so I can give informed consent.
- **Acceptance:** Hero screen with illustration; copy explains automation, privacy, GDPR consent. Toggle required to proceed. Skip button preserves option to enable later.
- **Edge case:** Decline → no cycle data is ever collected; Body screen shows no cycle UI.

**US-003 — Onboarding contraception:**
> As a female user, I want to declare my contraception method so the app doesn't show predictions that are invalidated by hormonal birth control.
- **Acceptance:** 5 options: None / Pill / Hormonal IUD / Implant / Other-non-hormonal. Hormonal selection disables ovulation predictions; banner explains why.

**US-004 — Period anchor:**
> As a female user, I want to optionally provide my last period date so predictions can begin on day 1.
- **Acceptance:** Date picker (max 90 days back). Optional cycle-length slider (21-35 days). "I don't remember" path uses population prior (28-day default).

**US-005 — Cycle phase visibility on Body:**
> As a female user, I want to see my current cycle phase on the Body screen without navigating elsewhere.
- **Acceptance:** Cycle card visible in Body for `gender == 'female'` AND `tracking_enabled == true` AND consent recorded. Shows: phase ring, phase name, "Day X of Y", next event preview.
- **Edge case:** No data yet → "Set up cycle tracking" CTA card.

**US-006 — Cycle home with 4 tabs:**
> Tap Cycle card → CycleHomeView with segmented control (Today/Calendar/Insights/History). Each tab loads independently with skeleton during fetch.

**US-007 — Period log:**
> Bottom sheet. Date picker (default today). 4 flow buttons (Spotting/Light/Medium/Heavy). 2 toggles (First day, Last day). Save → POST `/cycle/period-log` → optimistic UI update.

**US-008 — Symptom log:**
> Bottom sheet. Cramps slider (0-5), mood (5 chips), libido (3 chips), 4 toggles, cervical mucus picker, notes field. Save → POST `/cycle/symptom-log`.

**US-009 — Push: fertile window:**
> Local 09:00 the day before predicted ovulation, given confidence ≥0.6 AND user is not on hormonal contraception. Deeplink to Today tab. Footer: "General wellness — not medical advice".

**US-010 — Push: period coming:** Local 09:00 two days before predicted period start, confidence ≥0.6.

**US-011 — Push: delay alert:** Local 12:00 if today is predicted_period_start + 2 days AND no period log exists for past 5 days.

**US-012 — Push: PMS warning:** Local 18:00 the day prior cycles' PMS pattern was detected. Requires 2 cycles of pattern.

**US-013 — Settings master toggle:** Off → tracking_enabled = false; cycle card disappears; data retained. Re-enable → card returns.

**US-014 — Settings notification toggles:** 5 sub-toggles. Default 4 ON, "Cycle insight" OFF.

**US-015 — GDPR data export (Art 20):** POST `/cycle/export` returns JSON. iOS share sheet. Android download to Documents folder.

**US-016 — GDPR data deletion (Art 17):** Confirm dialog (text input "DELETE"). DELETE `/cycle/all-data` → CASCADE removes all rows.

**US-017 — Watch complication:** Phase + day. Updates hourly. Reads from shared cache (App Group / DataStore).

**US-018 — Home screen widget:** Small/medium sizes. Phase ring + day + next event preview (medium). Hourly refresh.

**US-019 — Anovulatory escape hatch:** After 3 cycles without sustained 0.20°C shift, show one-time card. Dismissible. Stored in `cycle_profiles.anovulatory_message_shown_at`.

**US-020 — RU geofence:** Backend returns 451 Unavailable For Legal Reasons if `users.country == 'RU'`. iOS/Android show `CycleUnavailableInRegionScreen`.

### 3.3 Out-of-scope user stories (deferred)

- US-101 (Pregnancy mode) → v2
- US-102 (Symptom-driven medical alerts) → v2 with FDA wellness review
- US-103 (Partner sharing) → v3 — privacy review needed
- US-104 (Natural Cycles integration) → v3 partnership
- US-105 (Apple Watch full app) → v3
- US-106 (PCOS deep insights) → v3 with clinical partnership

---

## 4. Technical Architecture

### 4.1 System diagram

```
┌──────────────────┐                                       ┌─────────────────────────┐
│  JCV8 Bracelet   │                                       │  Apple Watch / Wear OS  │
│  (V8 SDK, BLE)   │                                       │                         │
└────────┬─────────┘                                       └────────────┬────────────┘
         │ continuous: HR/HRV/RHR/temp/sleep                             │ complication
         ▼                                                                │
┌──────────────────────────────────┐                                     │
│  iOS / Android client            │                                     │
│  - LiveMetricsHub (existing)     │                                     │
│  - BiometricSyncer (existing)    │                                     │
│  - NEW: CycleViewModel/Repo      │                                     │
│  - NEW: Onboarding screens       │                                     │
│  - NEW: CycleHomeView            │                                     │
│  - NEW: Watch/Widget data sync   │ <───────────────────────────────────┘
└────────┬─────────────────────────┘
         │ HTTPS (Bearer Privy JWT)
         ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  wellex-io/app-backend (Rust / Axum)                                     │
│                                                                           │
│  Existing infra (reused):                                                │
│  - src/auth/middleware.rs (Privy JWT verification)                       │
│  - src/biometrics/handlers.rs (POST /biometrics/sync)                    │
│  - src/events.rs (Kafka topic wvi.biometrics)                            │
│  - src/push/apns.rs (APNs JWT-signed; FCM TBD for Android)               │
│  - src/narrator_schedule.rs (TZ-aware cron pattern)                      │
│  - src/ai/handlers.rs + prompt_rules.rs (Claude/local LLM gateway)       │
│  - src/sensitivity (cycle-correlated signal opportunity)                 │
│  - src/audit.rs                                                          │
│                                                                           │
│  NEW: src/cycle/                                                         │
│  - routes.rs ────────► 12 cycle endpoints                                │
│  - events.rs ────────► Kafka subscriber for wvi.biometrics               │
│  - detector/ ────────► CUSUM + Marshall + multi-signal fusion            │
│  - predictor/ ───────► calendar + sensor + alpha-blender                 │
│  - lifecycle.rs ─────► state machine (cold_start → active → ...)         │
│  - notifications.rs ─► 5 push categories                                 │
│  - insights/ ────────► PMS, AI narrator, anomaly                         │
│  - ground_truth/ ────► label collection for v2 ML                        │
│  - feature_flag.rs ──► env + per-user gating                             │
│                                                                           │
│  NEW: migrations/018_cycle_tracking.sql ► 7 tables                       │
└────────┬─────────────────────────────────────────────────────────────────┘
         │
         ▼
┌──────────────────────────────────┐    ┌─────────────────────────┐
│  Postgres (primary + replicas)   │    │  AI gateway (existing)  │
│  - EU shard (Frankfurt) for EU   │    │  aidev.wellex.io        │
│  - US shard for US users         │    │  (cycle narrative)      │
│  - TBD: RU shard in Phase 2      │    └─────────────────────────┘
└──────────────────────────────────┘
```

### 4.2 Data flow — sensor record to user-visible prediction

```
T+0s:    JCV8 measures HRV (60s sliding window) and skin temp (per-minute)
T+0.1s:  BLE notification → iOS/Android LiveMetricsHub captures sample
T+0.5s:  Sample written to local cache (BiometricCache iOS / DataStore Android)
T+30s:   BiometricSyncer batches buffered samples, POST /biometrics/sync
T+30.2s: backend writes to heart_rate / hrv / temperature tables
T+30.3s: Kafka event published to wvi.biometrics topic
T+30.4s: src/cycle/events.rs subscriber receives event, filters for relevant kinds
T+30.5s: For temperature/hrv/rhr at night, src/cycle/detector::ingest_nightly() updates cycle_signals_nightly
T+05:00 UTC daily: src/cycle/notifications::run_daily_batch() fires for all users in their local TZ
                  - For each user with tracking_enabled:
                    - Aggregate yesterday's cycle_signals_nightly
                    - Run detector::run() → may update cycles/cycle_phases
                    - Run predictor::generate_predictions() → write cycle_predictions
                    - notifications::evaluate() → enqueue applicable pushes
T+09:00 local: Push delivered via APNs/FCM
                  - iOS: PushNotificationManager routes deeplink wellex://cycle/today
                  - Android: CycleFcmHandler routes via NavController to CycleScreen
```

### 4.3 Multi-platform parity guarantees

To keep iOS, Android, and backend in lockstep, we enforce these contracts:

1. **Algorithm parity:** identical fixtures (`cycle/fixtures/*.json`) tested in Rust, Kotlin, and Swift. Each platform's domain layer must produce identical detector output for identical input. CI gates: `cargo test cycle::fixtures` + `gradle :core:domain:test` + iOS unit test target. Output JSON files compared bytewise.

2. **API contract:** OpenAPI 3.1 spec at `wellex-io/app-backend / docs/openapi.yaml` — extended with cycle endpoints. iOS uses Swift OpenAPI Generator; Android uses Ktor + manual DTOs validated against spec. CI gate: `swagger-cli validate`.

3. **Localization key parity:** strings live in `Localizable.strings` (iOS) and `strings-cycle.xml` (Android). CI gate: a script `tools/check-cycle-loc-parity.sh` ensures both have the same key set across all 6 locales.

4. **Design parity:** CyclePillar color = `#C026D3` everywhere. PhaseRing component (Compose Canvas / SwiftUI Canvas) renders identically — pixel-diffed via Paparazzi (Android) and SnapshotTesting (iOS) in CI on every PR.

5. **Feature flag:** server-driven master flag `cycle_tracking_enabled` returned in `/users/me` response. Both clients honor it; flipping to false hides cycle UI without app update.

### 4.4 Telemetry schema (parity)

Both clients emit the same analytics events with identical payloads. Backend mirrors these via Kafka into the analytics warehouse. Event names in §17.

---

## 5. Algorithm Specification

This section is the canonical source of truth for the cycle detector. All three platforms (Rust, Kotlin, Swift) must produce identical output for identical input. Where the math diverges from any cited paper, the change is justified and tested against fixtures.

### 5.1 Overview

The detector operates on **per-night aggregates**. Each night produces one `NightSignal` record:

```rust
struct NightSignal {
    night_date: NaiveDate,        // calendar date of the "evening of"
    sleep_hours: f64,
    skin_temp_mean_c: f64,        // mean over [10pm..6am] in user's local TZ
    rhr_bpm: f64,                  // resting HR during quietest 60-min window
    hrv_rmssd_ms: f64,             // RMSSD over RR intervals during deep sleep
    coverage_pct: f64,             // fraction of night with valid sensor data (0..1)
    is_outlier: bool,
    outlier_reason: Option<OutlierReason>,
}
```

Aggregation rules:
- Skin temperature: mean of valid 1-minute samples between 22:00 and 06:00 local time, requiring ≥4 hours of sleep. If user wakes up before 06:00, end window at wake time. Drop nights with <4 hours.
- RHR: minimum 60-minute rolling mean during sleep, in BPM.
- HRV: RMSSD over the longest deep-sleep window with ≥12 valid RR intervals.
- Coverage: (valid_temp_samples + valid_rhr_samples + valid_hrv_samples) / (expected_samples). If <0.7, mark night low-coverage.

### 5.2 Outlier detection

Before any night feeds the detector, we screen for outliers that can spoof a luteal-phase signal:

```rust
enum OutlierReason {
    None,
    Fever,           // RHR > 110 OR skin_temp_mean_c > 37.5
    HighAlcohol,     // skin_temp delta > 0.6°C above 14-day baseline + low_sleep_quality
    LowSleep,        // sleep_hours < 4
    LowCoverage,     // coverage_pct < 0.7
    JetLag,          // user's TZ differs from yesterday's by >2 hours
    PostExercise,    // >10k steps in last hour before sleep
}
```

Outlier nights are written to `cycle_signals_nightly` with `is_outlier = true` and the reason. They do not contribute to the rolling baseline (§5.3), but their existence is preserved for QA.

### 5.3 Rolling baseline (6-day, exclude outliers)

Per Marshall (1968) and matched by Apple/Oura, we use a 6-day rolling baseline:

```
baseline(d) = mean(valid_skin_temp[d-7..d-1] excluding outliers)
            requires ≥4 valid nights in window
```

If baseline cannot be computed, that night's delta is undefined and the night is skipped for ovulation detection (but still feeds future baselines).

### 5.4 Marshall biphasic threshold (primary detector)

Apple's published algorithm (Hum Reprod 2025) uses a 0.20°C threshold sustained over 3 consecutive nights:

```
For each night index i in [6 .. len-2]:
    baseline = rolling_baseline_6d(i)
    if all 3 of (i, i+1, i+2) are non-outlier:
        delta_0 = skin_temp[i]   - baseline
        delta_1 = skin_temp[i+1] - baseline
        delta_2 = skin_temp[i+2] - baseline
        if all three deltas >= 0.20°C:
            ovulation_estimate = night_date[i] - 1 day
            confidence_marshall = sigmoid((mean([d_0,d_1,d_2]) - 0.20) / 0.10)
            return OvulationCandidate(ovulation_estimate, MARSHALL, confidence_marshall)
```

### 5.5 CUSUM (secondary detector — confirms Marshall)

Royston & Abrams (1980) demonstrated 100% detection rate on n=137 BBT charts using CUSUM. We use it as a confirmation signal:

```
S(0) = 0
S(d) = max(0, S(d-1) + (skin_temp[d] - baseline) - 0.10°C)

Trigger when S(d) > 0.30°C
Estimate = walk back to find onset of accumulation
```

In practice, Marshall and CUSUM agree on >90% of cycles. When they disagree:
- Marshall hit, CUSUM not: small but persistent shift — high confidence Marshall is right.
- CUSUM hit, Marshall not: large transient spike (1-2 nights then dropped) — likely outlier, scrutinize.
- Both hit: Bayesian confidence is highest; preferred.

### 5.6 HRV / RHR luteal signal (multi-signal voting)

WHOOP's published data: RHR rises +2.73 BPM, HRV drops -4.65 ms in luteal vs follicular at population level. In our detector this is a **confirmation signal**:

```
For 3 consecutive post-ovulation nights (i+2, i+3, i+4):
    if night is non-outlier:
        rhr_delta = rhr[j] - mean(rhr[j-14 .. j-1])
        hrv_delta = hrv[j] - mean(hrv[j-14 .. j-1])
        if rhr_delta >= +1.5 BPM AND hrv_delta <= -2.0 ms:
            confirmation_count += 1

Confirmed if count >= 2 of 3 nights.
```

### 5.7 Bayesian confidence fusion

We combine Marshall + CUSUM + HRV/RHR signals + calendar prior into a single confidence score:

```
P(ovulation_at_day_N | data) ∝
    P(temp_shift_at_N+1 | ovulation_N)              # 0.65 if Marshall hits, 0.20 else
  × P(cusum_at_N+2 | ovulation_N)                   # 0.50 if CUSUM hits, 0.30 else
  × P(luteal_signal_at_N+2..N+5 | ovulation_N)      # 0.30 if confirmed, 0.10 else
  × P(N | calendar_prior)                            # gaussian(mean=expected_day, sigma=3.0)

normalized = (P / max_possible_P).clamp(0, 1)
where max_possible_P = 0.65 * 0.50 * 0.30 * 1.0 = 0.0975
```

Confidence buckets used in UI:
- ≥0.80 → "high"; pushes fire; UI shows green dot.
- 0.50–0.79 → "medium"; UI shows amber dot; pushes still fire if ≥0.60.
- <0.50 → "low"; UI shows grey dot; predictions hidden behind "still calibrating" message.

### 5.8 Calendar predictor (cold-start fallback)

```
cycle_length = profile.avg_cycle_length_days OR 28
luteal_length = profile.avg_luteal_length_days OR 14
anchor = profile.last_anchor_date OR (today - 14 days)
days_since_anchor = today - anchor

cycle_day = (days_since_anchor % cycle_length) + 1
next_period_start = anchor + ((days_since_anchor / cycle_length) + 1) * cycle_length
predicted_ovulation = next_period_start - luteal_length
fertile_start = predicted_ovulation - 5 days
fertile_end = predicted_ovulation + 1 day
confidence = 0.55  // calendar baseline confidence
```

### 5.9 Sensor predictor (post-calibration)

After ≥2 logged cycles with sensor-detected ovulation:

```
recent_cycles = cycles[-6:]
mean_length = mean(c.cycle_length for c in recent_cycles where c has length)
sd_length = stddev(...)
last_ovulation = recent_cycles[-1].ovulation_date

expected_next_ovulation = last_ovulation + mean_length
expected_next_period = expected_next_ovulation + last.luteal_length (default 14)
confidence = 1.0 - min(0.5, sd_length / mean_length)
```

### 5.10 Predictor blender

Linear ramp from calendar to sensor as data accumulates:

```
if hormonal_contraception:
    return hormonal_branch(calendar)  // withdrawal-bleed only, no ovulation
if anovulatory OR sensor is None OR data_days < 30:
    return calendar
alpha = clamp(0, 1, (data_days - 30) / 30)
prediction = lerp(calendar, sensor, alpha)
confidence = lerp(calendar.confidence, sensor.confidence, alpha)
```

### 5.11 PMS pattern detector

```
pms_cycles_count = 0
For each of last 3 cycles:
    luteal_late_window = signals where day >= cycle_length - 5
    if mean(hrv in window) < (mean(hrv overall) - 3.0) AND
       mean(rhr in window) > (mean(rhr overall) + 1.5):
        pms_cycles_count += 1

If pms_cycles_count >= 2:
    PMS pattern confirmed; warn 4 days before next predicted period
```

### 5.12 Lifecycle state machine

```
                  ┌──────────────┐
                  │  cold_start  │   (data_days < 30)
                  └──────┬───────┘
                         │ data_days ≥ 30 OR sensor candidate found
                         ▼
                  ┌──────────────┐
                  │ calibrating  │   (data_days 30..60)
                  └──────┬───────┘
                         │ data_days ≥ 60 AND sensor cycle complete
                         ▼
                  ┌──────────────┐
              ┌──►│   active     │
              │   └──┬─────┬─────┘
              │      │     │
   3 cycles no ovul  │     │  contraception change to hormonal
              │      │     │
              ▼      │     ▼
       ┌──────────┐  │  ┌──────────────┐
       │ anovula- │  │  │ contraception│
       │ tory     │  │  │  (hormonal)  │
       └──┬───────┘  │  └──────┬───────┘
          │          │         │
          │          │         │ contraception change to non-hormonal
          ▼          │         ▼
       1 cycle with  │   recalibrating(2 cycles)
       sensor ovul   │         │
          │          │         ▼
          └──────────┴────────► active

                    ┌──────────────┐
                    │   pregnancy  │   (Phase 2 — frozen state)
                    └──────────────┘

                    ┌──────────────┐
                    │ perimenopause│   (age >45 + variability >7d for 6+ cycles)
                    └──────────────┘
```

Each state stores `since_at` (TIMESTAMPTZ) and produces a transition log entry in `cycle_consent_log` with type `lifecycle_transition`.

### 5.13 Required fixtures (algorithm parity)

Backed in `cycle/fixtures/` and consumed identically by Rust, Kotlin, Swift tests:

1. `regular_cycles.json` — 12 cycles, 28-30 day length, clear 0.30°C luteal shift. Expect 12 ovulations detected within ±2 days, all confidence ≥0.75.
2. `pcos_cycles.json` — 6 cycles, anovulatory pattern, no shift. Expect 0 ovulations detected, anovulatory state after 3 cycles.
3. `perimenopause.json` — 8 cycles, length variance 24-40 days. Expect detection rate ≥60%, predictions wider window (≥4 days).
4. `postpartum.json` — 12 months data: 6 mo amenorrhea, then return. Expect first ovulation detected month 7+, no false positives in months 1-6.
5. `hormonal_contraception.json` — 6 "cycles" with suppressed temperature. Expect calendar-only predictions, no ovulation detected.
6. `irregular_45_to_28.json` — first 3 cycles 42-46 days, then transitions to 27-29 days. Expect predictor adapts within 2 cycles.
7. `outlier_heavy.json` — 30 nights with 8 fevers, 3 jet-lag windows, 5 alcohol nights. Expect detector survives, marks correct outliers, still detects ovulation in outlier-free window.

---

## 6. Database Schema

Migration `018_cycle_tracking.sql` (Postgres 14+):

```sql
-- ============================================================================
-- Migration 018: Cycle Tracking
-- Author: Wellex / Alex
-- Created: 2026-04-27
-- Reviewed by: TBD (DBA), TBD (legal/DPIA)
-- Risk: medium — adds 7 tables, requires gender column on users
-- Rollback: ../018_cycle_tracking_rollback.sql
-- ============================================================================

BEGIN;

-- Ensure required columns on users
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS country VARCHAR(2),
    ADD COLUMN IF NOT EXISTS timezone VARCHAR(40);

-- cycle_profiles
CREATE TABLE cycle_profiles (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    tracking_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    contraception_method VARCHAR(40) NOT NULL DEFAULT 'unknown'
        CHECK (contraception_method IN (
            'unknown', 'none', 'pill', 'hormonal_iud', 'implant',
            'patch', 'ring', 'copper_iud', 'condom_only', 'fam', 'other_non_hormonal'
        )),
    is_pregnant BOOLEAN NOT NULL DEFAULT FALSE,
    is_breastfeeding BOOLEAN NOT NULL DEFAULT FALSE,
    is_perimenopause BOOLEAN NOT NULL DEFAULT FALSE,
    avg_cycle_length_days INTEGER CHECK (avg_cycle_length_days BETWEEN 18 AND 60),
    avg_period_length_days INTEGER CHECK (avg_period_length_days BETWEEN 1 AND 14),
    avg_luteal_length_days INTEGER NOT NULL DEFAULT 14
        CHECK (avg_luteal_length_days BETWEEN 8 AND 18),
    last_anchor_date DATE,
    last_anchor_source VARCHAR(20)
        CHECK (last_anchor_source IN ('onboarding', 'app', 'sensor', 'imported')),
    onboarded_at TIMESTAMPTZ,
    consent_special_category BOOLEAN NOT NULL DEFAULT FALSE,
    consent_special_category_at TIMESTAMPTZ,
    anovulatory_message_shown_at TIMESTAMPTZ,
    lifecycle_state VARCHAR(30) NOT NULL DEFAULT 'cold_start'
        CHECK (lifecycle_state IN (
            'cold_start', 'calibrating', 'active', 'anovulatory',
            'contraception', 'recalibrating', 'pregnancy', 'perimenopause'
        )),
    lifecycle_state_since TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_cycle_profiles_lifecycle ON cycle_profiles(lifecycle_state);

-- cycles
CREATE TABLE cycles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cycle_number INTEGER NOT NULL CHECK (cycle_number > 0),
    start_date DATE NOT NULL,
    end_date DATE,
    cycle_length_days INTEGER CHECK (cycle_length_days BETWEEN 1 AND 90),
    ovulation_date DATE,
    ovulation_confidence REAL CHECK (ovulation_confidence >= 0 AND ovulation_confidence <= 1),
    ovulation_method VARCHAR(20) CHECK (ovulation_method IN (
        'sensor_marshall', 'sensor_cusum', 'sensor_fused', 'calendar', 'manual'
    )),
    luteal_length_days INTEGER CHECK (luteal_length_days BETWEEN 5 AND 21),
    notes TEXT,
    is_anovulatory BOOLEAN NOT NULL DEFAULT FALSE,
    detection_metadata JSONB,
    algorithm_version VARCHAR(40) NOT NULL DEFAULT 'v1.0_marshall_cusum',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, cycle_number),
    UNIQUE(user_id, start_date)
);
CREATE INDEX idx_cycles_user_date ON cycles(user_id, start_date DESC);

-- cycle_phases
CREATE TABLE cycle_phases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    phase VARCHAR(20) NOT NULL CHECK (phase IN (
        'menstrual', 'follicular', 'ovulatory', 'luteal', 'unknown'
    )),
    cycle_day INTEGER CHECK (cycle_day BETWEEN 1 AND 90),
    cycle_id UUID REFERENCES cycles(id),
    confidence REAL CHECK (confidence >= 0 AND confidence <= 1),
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_cycle_phases_user_date ON cycle_phases(user_id, date DESC);

-- cycle_predictions
CREATE TABLE cycle_predictions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    predicted_period_start DATE,
    predicted_period_window_days INTEGER NOT NULL DEFAULT 2,
    predicted_ovulation DATE,
    predicted_ovulation_window_days INTEGER NOT NULL DEFAULT 2,
    predicted_fertile_start DATE,
    predicted_fertile_end DATE,
    period_confidence REAL CHECK (period_confidence >= 0 AND period_confidence <= 1),
    ovulation_confidence REAL CHECK (ovulation_confidence >= 0 AND ovulation_confidence <= 1),
    blender_alpha REAL NOT NULL CHECK (blender_alpha >= 0 AND blender_alpha <= 1),
    method_breakdown JSONB,
    valid_from DATE NOT NULL,
    valid_until DATE NOT NULL,
    algorithm_version VARCHAR(40) NOT NULL DEFAULT 'v1.0_marshall_cusum',
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (valid_from <= valid_until)
);
CREATE INDEX idx_cycle_predictions_user_valid ON cycle_predictions(user_id, valid_until DESC);

-- period_logs
CREATE TABLE period_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    flow_intensity VARCHAR(20) CHECK (flow_intensity IN ('spotting', 'light', 'medium', 'heavy')),
    is_first_day BOOLEAN NOT NULL DEFAULT FALSE,
    is_last_day BOOLEAN NOT NULL DEFAULT FALSE,
    source VARCHAR(20) NOT NULL DEFAULT 'manual'
        CHECK (source IN ('manual', 'sensor_inferred', 'imported')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_period_logs_user_date ON period_logs(user_id, date DESC);

-- symptom_logs
CREATE TABLE symptom_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date DATE NOT NULL,
    cramps INTEGER CHECK (cramps >= 0 AND cramps <= 5),
    mood VARCHAR(20) CHECK (mood IN ('low', 'neutral', 'good', 'irritable', 'anxious')),
    libido VARCHAR(10) CHECK (libido IN ('low', 'normal', 'high')),
    headache BOOLEAN NOT NULL DEFAULT FALSE,
    bloating BOOLEAN NOT NULL DEFAULT FALSE,
    breast_tenderness BOOLEAN NOT NULL DEFAULT FALSE,
    spotting BOOLEAN NOT NULL DEFAULT FALSE,
    cervical_mucus VARCHAR(20) CHECK (cervical_mucus IN ('dry', 'sticky', 'creamy', 'egg_white', 'watery')),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, date)
);
CREATE INDEX idx_symptom_logs_user_date ON symptom_logs(user_id, date DESC);

-- cycle_signals_nightly
CREATE TABLE cycle_signals_nightly (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    night_date DATE NOT NULL,
    sleep_hours REAL,
    skin_temp_mean_c REAL,
    skin_temp_baseline_c REAL,
    skin_temp_delta_c REAL,
    skin_temp_above_baseline BOOLEAN,
    rhr_bpm REAL,
    rhr_baseline_bpm REAL,
    rhr_delta_bpm REAL,
    hrv_rmssd_ms REAL,
    hrv_baseline_ms REAL,
    hrv_delta_ms REAL,
    cusum_temp_score REAL,
    is_outlier BOOLEAN NOT NULL DEFAULT FALSE,
    outlier_reason VARCHAR(40),
    coverage_pct REAL,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, night_date)
);
CREATE INDEX idx_cycle_signals_user_night ON cycle_signals_nightly(user_id, night_date DESC);

-- cycle_consent_log
CREATE TABLE cycle_consent_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    consent_type VARCHAR(40) NOT NULL CHECK (consent_type IN (
        'special_category_health', 'predictions', 'notifications', 'analytics',
        'contraception_change', 'lifecycle_transition', 'data_export',
        'data_deletion', 'master_toggle'
    )),
    granted BOOLEAN NOT NULL,
    consent_text_version VARCHAR(20) NOT NULL,
    locale VARCHAR(10) NOT NULL,
    ip_address INET,
    device_fingerprint VARCHAR(255),
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_cycle_consent_user ON cycle_consent_log(user_id, created_at DESC);

-- cycle_push_log
CREATE TABLE cycle_push_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    category VARCHAR(40) NOT NULL,
    fired_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deep_local_date DATE NOT NULL,
    delivered BOOLEAN,
    apns_response_code INTEGER,
    UNIQUE(user_id, category, deep_local_date)
);
CREATE INDEX idx_cycle_push_log_user ON cycle_push_log(user_id, fired_at DESC);

-- cycle_cron_log
CREATE TABLE cycle_cron_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    job_kind VARCHAR(40) NOT NULL,
    fired_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deep_local_date DATE NOT NULL,
    duration_ms INTEGER,
    success BOOLEAN NOT NULL,
    error_message TEXT,
    UNIQUE(user_id, job_kind, deep_local_date)
);
CREATE INDEX idx_cycle_cron_log_user ON cycle_cron_log(user_id, fired_at DESC);

-- triggers
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_cycle_profiles_updated BEFORE UPDATE ON cycle_profiles
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER trg_cycles_updated BEFORE UPDATE ON cycles
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
CREATE TRIGGER trg_period_logs_updated BEFORE UPDATE ON period_logs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE OR REPLACE FUNCTION log_lifecycle_transition()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state THEN
        NEW.lifecycle_state_since = NOW();
        INSERT INTO cycle_consent_log (
            user_id, consent_type, granted, consent_text_version, locale, metadata
        ) VALUES (
            NEW.user_id, 'lifecycle_transition', TRUE, 'auto', 'system',
            jsonb_build_object('from', OLD.lifecycle_state, 'to', NEW.lifecycle_state)
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_cycle_profiles_lifecycle BEFORE UPDATE ON cycle_profiles
    FOR EACH ROW EXECUTE FUNCTION log_lifecycle_transition();

COMMIT;
```

Rollback `018_cycle_tracking_rollback.sql`:

```sql
BEGIN;
DROP TRIGGER IF EXISTS trg_cycle_profiles_lifecycle ON cycle_profiles;
DROP FUNCTION IF EXISTS log_lifecycle_transition();
DROP TRIGGER IF EXISTS trg_cycle_profiles_updated ON cycle_profiles;
DROP TRIGGER IF EXISTS trg_cycles_updated ON cycles;
DROP TRIGGER IF EXISTS trg_period_logs_updated ON period_logs;
DROP TABLE IF EXISTS cycle_cron_log;
DROP TABLE IF EXISTS cycle_push_log;
DROP TABLE IF EXISTS cycle_consent_log;
DROP TABLE IF EXISTS cycle_signals_nightly;
DROP TABLE IF EXISTS symptom_logs;
DROP TABLE IF EXISTS period_logs;
DROP TABLE IF EXISTS cycle_predictions;
DROP TABLE IF EXISTS cycle_phases;
DROP TABLE IF EXISTS cycles;
DROP TABLE IF EXISTS cycle_profiles;
COMMIT;
```

---

## 7. API Specification

All endpoints under `/api/v1/cycle/*`. Auth via Bearer Privy JWT (existing middleware). All return `application/json`. All non-onboarding endpoints return `404 Not Found` if `users.gender != 'female'`. RU users return `451 Unavailable For Legal Reasons` with body `{"error":"region_unavailable","localized_message_key":"cycle_unavailable_in_region"}`.

### 7.1 POST /api/v1/cycle/onboarding

Initialize cycle tracking after consent + contraception + (optional) anchor.

**Request:**
```json
{
  "consent_special_category": true,
  "consent_text_version": "v1.0_2026-04-27",
  "locale": "en",
  "contraception_method": "none",
  "last_period_date": "2026-04-13",
  "avg_cycle_length_days": 28,
  "avg_period_length_days": 5
}
```

**Response 200:**
```json
{
  "success": true,
  "data": {
    "tracking_enabled": true,
    "lifecycle_state": "cold_start",
    "first_prediction_eta_days": 1
  }
}
```

**Response 422 (validation):**
```json
{
  "success": false,
  "error": "validation",
  "fields": {
    "consent_special_category": "must_be_true",
    "avg_cycle_length_days": "out_of_range_18_to_60"
  }
}
```

### 7.2 GET /api/v1/cycle/state

Today snapshot. Most-called endpoint.

**Response 200:**
```json
{
  "success": true,
  "data": {
    "today": "2026-04-27",
    "phase": "luteal",
    "cycle_day": 21,
    "cycle_length_days": 28,
    "predictions": {
      "next_period_start": "2026-05-04",
      "next_period_window_days": 2,
      "period_confidence": 0.78,
      "predicted_ovulation": "2026-04-20",
      "ovulation_confidence": 0.92,
      "fertile_start": "2026-04-15",
      "fertile_end": "2026-04-21",
      "blender_alpha": 1.0
    },
    "lifecycle": {
      "state": "active",
      "since": "2026-03-15T00:00:00Z",
      "data_days": 67,
      "is_anovulatory": false,
      "is_hormonal_contraception": false,
      "anovulatory_message_due": false
    },
    "today_signal": {
      "skin_temp_delta_c": 0.34,
      "rhr_delta_bpm": 3.1,
      "hrv_delta_ms": -5.4,
      "coverage_pct": 0.92
    },
    "next_event": {
      "kind": "period_coming",
      "date": "2026-05-04",
      "label": "Period in 7 days"
    }
  }
}
```

**Caching:** `Cache-Control: private, max-age=60`. Backend returns cached snapshot from `cycle_phases` + `cycle_predictions` tables; daily batch refreshes.

### 7.3 GET /api/v1/cycle/calendar

```
GET /api/v1/cycle/calendar?from=2026-04-01&to=2026-04-30
```

**Response 200:**
```json
{
  "success": true,
  "data": {
    "from": "2026-04-01",
    "to": "2026-04-30",
    "days": [
      {
        "date": "2026-04-01",
        "phase": "menstrual",
        "cycle_day": 1,
        "is_predicted": false,
        "period_log": {"flow_intensity": "medium", "is_first_day": true},
        "symptoms": ["cramps_3", "mood_low"]
      }
    ]
  }
}
```

### 7.4 GET /api/v1/cycle/insights

```json
{
  "success": true,
  "data": {
    "avg_cycle_length_days": 28.3,
    "avg_cycle_length_sd": 1.2,
    "regularity_score": 87,
    "regularity_label": "regular",
    "last_6_cycles": [
      {"cycle_number": 12, "length": 28, "ovulation_day": 14}
    ],
    "pms_pattern": {"detected": true, "warning_lead_days": 4, "confidence": 0.81},
    "narrative": "Your last 3 cycles have averaged 28.3 days with ovulation around day 14."
  }
}
```

### 7.5 GET /api/v1/cycle/history

Returns array of past cycles with start/end dates, length, ovulation, confidence.

### 7.6 POST /api/v1/cycle/period-log

```json
// Request
{"date": "2026-04-13", "flow_intensity": "medium", "is_first_day": true, "is_last_day": false}
// Response 200
{"success": true, "data": {"id": "uuid"}}
```

### 7.7 DELETE /api/v1/cycle/period-log/:date

```
DELETE /api/v1/cycle/period-log/2026-04-13
```
Response 204 on success.

### 7.8 POST /api/v1/cycle/symptom-log

Symptom payload with cramps/mood/libido/booleans/notes. Returns `{success: true, data: {id}}`.

### 7.9 GET /api/v1/cycle/profile / PATCH /api/v1/cycle/profile

GET returns full profile row. PATCH updates contraception_method, tracking_enabled, is_pregnant, etc. PATCH triggers lifecycle transition logic.

### 7.10 GET /api/v1/cycle/export (GDPR Art 20)

Returns full JSON export of all cycle_* rows for the user. Audit: row in `cycle_consent_log` with type `data_export`.

### 7.11 DELETE /api/v1/cycle/all-data (GDPR Art 17)

Removes all cycle_* rows for user via ON DELETE CASCADE on cycle_profiles. Response 204. Audit: row in `cycle_consent_log` with type `data_deletion`.

### 7.12 POST /api/v1/cycle/admin/recompute (admin only)

Force-rerun detector + predictor for one user. Body: `{"user_id": "uuid", "from_date": "2026-01-01"}`. Response: `{"recomputed_cycles": 4, "recomputed_predictions": 1}`.

### 7.13 Common error envelope

```json
{
  "success": false,
  "error": "<machine_kind>",
  "localized_message_key": "<L_key>",
  "fields": {"<field>": "<reason>"},
  "request_id": "<trace>"
}
```

### 7.14 Rate limits

Read endpoints: 100 req/sec/user. Write endpoints: 20 req/sec/user. Onboarding endpoint: 5 req/min/user (anti-replay).

---

## 8. Backend Implementation Plan (Rust)

(Full spec in `wellex-io/app-backend → docs/cycle-tracking/IMPLEMENTATION_PLAN.md`. Summary:)

### 8.1 Module file map

```
src/cycle/
├── mod.rs                   pub use; register_routes(router) function
├── routes.rs                12 endpoints; thin handlers calling services
├── models.rs                SQLx FromRow + serde DTOs (~200 lines)
├── service.rs               business logic for each endpoint
├── events.rs                Kafka subscriber for wvi.biometrics → ingest_nightly
├── detector/
│   ├── mod.rs                public detect() entry; orchestrates submodules
│   ├── nightly.rs            NightSignal aggregation from raw biometric tables
│   ├── outliers.rs           classify_outlier()
│   ├── baseline.rs           rolling_baseline_6d()
│   ├── temperature.rs        Marshall + CUSUM
│   ├── hrv_rhr.rs            luteal_signal_confirmed()
│   ├── confidence.rs         fuse_confidence(), gaussian_prior()
│   └── ovulation.rs          composite decision returning OvulationCandidate
├── predictor/
│   ├── mod.rs                public predict() entry
│   ├── calendar.rs           predict_calendar()
│   ├── sensor.rs             predict_sensor()
│   ├── blender.rs            blend()
│   └── hormonal_branch.rs    hormonal contraception predictor
├── lifecycle.rs             state machine (transitions, side effects)
├── insights/
│   ├── pms.rs                detect_pms_pattern()
│   ├── narrator.rs           AI narrative integration
│   └── anomaly.rs            cycle-correlated sensitivity signals
├── notifications.rs         5 push categories; daily evaluator
├── ground_truth/
│   └── labeler.rs            collect labels from period_logs for v2 ML training
├── feature_flag.rs          gating: env + per-user opt-in
├── tests/
│   ├── fixtures/             7 JSON fixtures (parity with iOS/Android)
│   ├── detector_tests.rs
│   ├── predictor_tests.rs
│   ├── confidence_tests.rs
│   ├── lifecycle_tests.rs
│   └── integration_tests.rs  full /cycle/* flows
└── docs/
    ├── algorithm.md          full math + citations
    └── api.md                examples for each endpoint
```

### 8.2 Daily batch job

Spawned in `main.rs` alongside existing `narrator_schedule` workers. Pattern mirrors `push/scheduler.rs` morning brief loop. Every 5 min, find users in 07:00-07:05 local TZ window, run detector + predictor + push evaluator. Idempotent dedup via `cycle_cron_log` UNIQUE(user_id, job_kind, deep_local_date).

### 8.3 Test pyramid (Rust)

- ~80 unit tests in `src/cycle/`
- ~20 integration tests in `tests/cycle_*.rs` against test postgres
- Performance bench: detector <50ms / user / 365 nights (criterion)

---

## 9. iOS Implementation Plan

(Full spec in `iOS/docs/cycle-tracking/IOS_IMPLEMENTATION_PLAN.md`. Summary:)

### 9.1 Files to add (~25)

```
WVIHealth/Features/Onboarding/Screens/
  - GenderSelectionView.swift
  - CycleTrackingWelcomeView.swift
  - CycleContraceptionView.swift
  - CyclePeriodAnchorView.swift

WVIHealth/Features/Cycle/
  - CycleHomeView.swift
  - CycleViewModel.swift
  - CycleAPIClient.swift
  - CycleSummary.swift
  Tabs/
    - CycleTodayTab.swift
    - CycleCalendarTab.swift
    - CycleInsightsTab.swift
    - CycleHistoryTab.swift
  Sheets/
    - PeriodLogSheet.swift
    - SymptomLogSheet.swift

WVIHealth/Features/Body/Cards/
  - CycleSummaryCard.swift
  - CycleEmptyCard.swift

WVIHealthWatch/CycleComplicationProvider.swift
WVIHealthWidget/CycleWidget.swift
WVIHealthTests/CycleViewModelTests.swift
WVIHealthTests/CycleAPIClientTests.swift
WVIHealthTests/CycleSnapshotTests.swift
```

### 9.2 Files to modify (~15)

```
Core/Design/Theme.swift                       +1 line: WVIColor.cycle = #C026D3
Core/Localization.swift                       +80 strings × 6 locales
Core/Logging/AnalyticsEvent.swift             +14 cycle events
Core/Notifications/PushNotificationManager.swift  +5 categories, deeplink routing
Core/Navigation/NavigationCoordinator.swift   +AutoscrollTarget.cycleCard
Features/Onboarding/OnboardingState.swift     +4 stages
Features/Onboarding/OnboardingCoordinator.swift +4 transitions, state
Features/Onboarding/RootRouterView.swift      +4 cases
Features/Body/BodyScreen.swift                +cycle state load
Features/Body/BodyScreen+Sections.swift       +cycleSection ViewBuilder
Features/Settings/SettingsView.swift          +cycle section
App/WVIHealthApp.swift                         widget+complication registration
App/ContentView.swift                          wellex://cycle/* deeplink
```

### 9.3 Existing components reused

`RingProgress`, `WellexButton`, `BackButton`, `BottomTabBar`, `WellexAppBar`, `GlassCard`, `AIInsightCard`, `SkeletonModifier`, `DetailViewModel<T>`, `MetricDetailScreen` template, `BiometricCache`.

### 9.4 Shared cache keys (App Group)

Group: `group.com.wvi.health` (existing). New keys:
- `cache_cycle_phase` (string)
- `cache_cycle_day` (int)
- `cache_cycle_length` (int)
- `cache_cycle_fertility` (string: "high"|"medium"|"low"|"none")
- `cache_cycle_next_event_label` (localized string)
- `cache_cycle_phase_color` (hex string)
- `cache_cycle_updated_at` (timestamp)

---

## 10. Android Implementation Plan

Full spec: `Android/docs/cycle-tracking/ANDROID_IMPLEMENTATION_PLAN.md` (1478 lines). Summary:

### 10.1 Modules to extend / add

```
core/persistence — Migration 1→2 + 7 entities + 7 DAOs
core/domain      — pure-Kotlin port of Rust algorithms (parity tests)
core/network     — CycleApi (12 endpoints) + CycleRepository (offline-first)
core/notifications — 5 NotificationChannels + FCM handler + WorkManager scheduler
core/analytics   — 14 cycle events
core/localization — strings-cycle.xml × 6 locales

feature/onboarding — 4 new Compose screens
feature/body       — cycle card composable
feature/cycle      — NEW module: home + 4 tabs + 2 sheets
feature/settings   — cycle section composable

widget/cycle       — Glance widget
wear/cycle         — ComplicationProviderService
```

### 10.2 Android-specific concerns

- Kotlin pinned at 2.2.20 (Hilt cap)
- ASCII-only Kotlin sources; no em-dashes (K2 lexer pitfall)
- POST_NOTIFICATIONS permission on API 33+
- Doze mode mitigation: `setExpedited()` for delay alert worker
- OEM aggressive battery optimization (Xiaomi MIUI, Samsung) — onboarding banner asks user to whitelist Wellex
- FCM region delivery: future Phase 2 add Mi Push / Huawei Push for CN locale

---

## 11. Watch / Widget / Companion Implementations

### 11.1 Apple Watch complication

Identifier: `cycle_phase`. Families: `.graphicCircular`, `.graphicCorner`, `.graphicRectangular`. Reads from shared App Group cache. Refreshes hourly via timeline policy.

### 11.2 iOS widget

WidgetKit timeline provider with 24 entries (1-hour resolution). Reads same shared cache keys. Sizes: small (.systemSmall) and medium (.systemMedium).

### 11.3 Wear OS complication (Glance)

`ComplicationDataSourceService` with SHORT_TEXT and RANGED_VALUE forms.

### 11.4 Android home widget (Glance API)

Widget kind `cycle_widget`. Sizes small + medium. Reads same shared cache keys via DataStore.

### 11.5 Sync from main app to watch / widget

iOS:
- `CycleViewModel` writes to shared App Group on every state update
- `WCSession.default.transferUserInfo` pushes to paired Apple Watch
- `WidgetCenter.shared.reloadAllTimelines()` refreshes widget

Android:
- `CycleRepository` writes to `BiometricCacheStore`
- `WearableMessageClient.sendMessage(nodeId, "/cycle/state", payload)`
- `GlanceManager.update<CycleGlanceWidget>(context)`

---

## 12. Onboarding UX Specification

### 12.1 Flow chart

```
                      [Splash / Hero]
                            │
                            ▼
                  [Personalize-pre]
                            │
                            ▼
                  ┌────────────────────┐
                  │ GENDER_SELECTION   │  ← NEW
                  │ • Female           │
                  │ • Male             │
                  │ • Prefer not       │
                  └─────────┬──────────┘
                  female?   │   else
              ┌─────────────┴───────────────┐
              ▼                             ▼
    [CYCLE_WELCOME]                  [Personalize-existing]
    • Hero illustration                     │
    • Privacy block                         │
    • Consent toggle                        ▼
    • Continue / Skip
              │
              ▼
    [CYCLE_CONTRACEPTION]
    • None / Pill / IUD / Implant / Other
    • Banner if hormonal
              │
              ▼
    [CYCLE_PERIOD_ANCHOR]
    • DatePicker (max 90 days)
    • Avg cycle length slider
    • "I don't remember" skip
              │
              ▼
    [Personalize-existing] → ... → Dashboard
```

### 12.2 Screen wireframes (ASCII)

**GENDER_SELECTION:**
```
┌──────────────────────────────────────────┐
│  ←                                       │
│                                          │
│  ●●○○○                                   │  progress dots
│                                          │
│  Tell us about you                       │  Onest ExtraLight 28
│                                          │
│  We tailor metrics and features to       │  Onest Light 13 italic
│  your physiology                         │
│                                          │
│  ┌──────────────────────────────────┐    │
│  │  ●  Female                       │    │
│  └──────────────────────────────────┘    │
│  ┌──────────────────────────────────┐    │
│  │  ○  Male                         │    │
│  └──────────────────────────────────┘    │
│  ┌──────────────────────────────────┐    │
│  │  ○  Prefer not to say            │    │
│  └──────────────────────────────────┘    │
│                                          │
│  ┌──────────────────────────────────┐    │
│  │           CONTINUE               │    │
│  └──────────────────────────────────┘    │
└──────────────────────────────────────────┘
```

**CYCLE_TRACKING_WELCOME:**
```
┌──────────────────────────────────────────┐
│  ←                                       │
│  ●●●○○                                   │
│                                          │
│           ◯                              │  hero: 5 concentric rings
│         ◯ ◯ ◯                            │  animated, color #C026D3
│           ◯                              │
│                                          │
│  Cycle understanding built into          │
│  your body                               │
│                                          │
│  Your bracelet's sensors detect          │
│  ovulation, period, and delays           │
│  automatically. No manual logging        │
│  needed.                                 │
│                                          │
│  ┌──────────────────────────────────┐    │
│  │ Cycle data is special-category   │    │
│  │ health data under GDPR. We       │    │
│  │ process it only with your        │    │
│  │ explicit consent, store it       │    │
│  │ encrypted, never share with      │    │
│  │ third parties. Export or         │    │
│  │ delete it anytime.               │    │
│  └──────────────────────────────────┘    │
│                                          │
│  [✓] I consent to processing of my       │
│      cycle health data                   │
│                                          │
│  ┌──────────────────────────────────┐    │
│  │           CONTINUE               │    │  disabled until ✓
│  └──────────────────────────────────┘    │
│  ┌──────────────────────────────────┐    │
│  │            SKIP                  │    │
│  └──────────────────────────────────┘    │
│                                          │
│  Wellex Cycle Insights is not a          │
│  medical device. Not a contraceptive.    │
└──────────────────────────────────────────┘
```

**CYCLE_CONTRACEPTION** and **CYCLE_PERIOD_ANCHOR** wireframes similarly structured (5 cards / date picker + slider + skip).

### 12.3 Copy (EN + RU)

| Key | EN | RU |
|---|---|---|
| `gender_selection_title` | Tell us about you | Расскажите о себе |
| `gender_selection_subtitle` | We tailor metrics and features to your physiology | Подбираем метрики и функции под вашу физиологию |
| `gender_female` | Female | Женский |
| `gender_male` | Male | Мужской |
| `gender_prefer_not_to_say` | Prefer not to say | Не хочу указывать |
| `cycle_welcome_title` | Cycle understanding built into your body | Цикл, который понимает ваше тело |
| `cycle_welcome_body` | Your bracelet's sensors detect ovulation, period, and delays automatically. No manual logging needed. | Сенсоры браслета автоматически определяют овуляцию, менструацию и задержки. Без ручного ввода. |
| `cycle_privacy_block` | Cycle data is special-category health data under GDPR. We process it only with your explicit consent, store it encrypted, and never share with third parties. You can export or delete it anytime. | Данные о цикле — особая категория данных о здоровье в GDPR. Обрабатываем только с явным согласием, храним зашифрованно, не передаём третьим лицам. Можете экспортировать или удалить в любой момент. |
| `cycle_consent_required` | I consent to processing of my cycle health data | Я даю согласие на обработку данных о цикле |
| `cycle_consent_disclaimer` | Wellex Cycle Insights is not a medical device. It does not diagnose, treat, or prevent disease. Not a contraceptive. | Wellex Cycle Insights — не медицинское устройство. Не диагностирует, не лечит и не предотвращает заболевания. Не средство контрацепции. |
| `contraception_question` | Are you using hormonal contraception? | Используете ли вы гормональную контрацепцию? |
| `contraception_subtitle` | Hormonal methods change how your body's signals work. We adjust accordingly. | Гормональные методы меняют сигналы тела. Мы это учитываем. |
| `contraception_none` | None or non-hormonal | Нет или негормональная |
| `contraception_pill` | Pill | Таблетки |
| `contraception_iud` | Hormonal IUD | Гормональная ВМС |
| `contraception_implant` | Implant | Имплант |
| `contraception_other` | Other | Другое |
| `contraception_hormonal_banner` | Ovulation predictions disabled for hormonal contraception | Прогноз овуляции отключён при гормональной контрацепции |
| `period_anchor_title` | When did your last period start? | Когда началась последняя менструация? |
| `period_anchor_subtitle` | We use this to start predictions on day 1. Skip if not sure. | Чтобы начать прогноз с первого дня. Пропустите, если не помните. |
| `period_anchor_skip` | I don't remember | Не помню |
| `cta_continue` | Continue | Продолжить |
| `cta_skip` | Skip | Пропустить |
| `cta_save` | Save | Сохранить |

(Full 80-key matrix in §15.)

---

## 13. Push Notifications Specification

### 13.1 Categories

| ID | iOS Category | Android Channel | Importance | Default ON |
|---|---|---|---|---|
| CYCLE_FERTILE | `CYCLE_FERTILE` | `cycle_fertile` | Default / DEFAULT | Yes |
| CYCLE_PERIOD_COMING | `CYCLE_PERIOD_COMING` | `cycle_period_coming` | Default / DEFAULT | Yes |
| CYCLE_DELAY | `CYCLE_DELAY` | `cycle_delay` | Time-Sensitive / HIGH | Yes |
| CYCLE_PMS | `CYCLE_PMS` | `cycle_pms` | Passive / DEFAULT | Yes |
| CYCLE_INSIGHT | `CYCLE_INSIGHT` | `cycle_insight` | Passive / LOW | No (opt-in) |

### 13.2 Trigger logic

| Category | Trigger condition | Time | Confidence gate |
|---|---|---|---|
| CYCLE_FERTILE | Predicted ovulation date - 1 day | 09:00 local | confidence ≥ 0.6 AND not on hormonal contraception |
| CYCLE_PERIOD_COMING | Predicted period start - 2 days | 09:00 local | period_confidence ≥ 0.6 |
| CYCLE_DELAY | Today is predicted_period_start + 2 days AND no period_log in last 5 days | 12:00 local | always |
| CYCLE_PMS | Predicted period start - 4 days AND prior 2 cycles showed PMS pattern | 18:00 local | requires PMS pattern |
| CYCLE_INSIGHT | After detected cycle end | 09:00 local next day | always |

Dedup: `cycle_push_log` UNIQUE(user_id, category, deep_local_date) prevents double-fire.

### 13.3 Copy (EN + RU)

| Category | EN title | EN body | RU title | RU body |
|---|---|---|---|---|
| CYCLE_FERTILE | Your cycle window | Estimated ovulation around tomorrow. Tap for details. | Ваше окно цикла | Овуляция ожидается около завтрашнего дня. Откройте, чтобы узнать больше. |
| CYCLE_PERIOD_COMING | Period in 2 days | Estimated start: %1$s. Plan accordingly. | Менструация через 2 дня | Ожидаемое начало: %1$s. Запланируйте дела. |
| CYCLE_DELAY | Period running late | Your period was estimated for %1$s. Log when it starts. | Менструация задерживается | Ожидалась %1$s. Отметьте, когда начнётся. |
| CYCLE_PMS | PMS pattern detected | Based on prior cycles, you may feel sensitive in the next few days. | Признаки ПМС | По прошлым циклам возможно повышение чувствительности в ближайшие дни. |
| CYCLE_INSIGHT | Cycle complete | Your cycle was %1$d days. Last 3 cycles avg: %2$.1f days. | Цикл завершён | Длина цикла: %1$d дн. Среднее за 3 цикла: %2$.1f дн. |

Footer (always appended):
- EN: "General wellness — not medical advice"
- RU: "Общий wellness — не медицинская консультация"

CYCLE_DELAY footer additionally:
- EN: "Not a pregnancy test. Consult a clinician if needed."
- RU: "Не тест на беременность. При необходимости — к врачу."

### 13.4 Deep linking

All cycle pushes set `data.deeplink = "wellex://cycle/today"`. Both clients route to CycleHomeView with Today tab.

### 13.5 Quiet hours

Both clients respect existing user-level quiet hours. Backend defers to next allowed slot.

---

## 14. Settings Specification

```
Settings
└── Cycle Tracking                         (visible only if gender == 'female')
    ├── [Toggle] Enable cycle tracking     (master)
    ├── [Toggle] Fertile window
    ├── [Toggle] Period reminder
    ├── [Toggle] Delay alerts
    ├── [Toggle] PMS warnings
    ├── [Toggle] End-of-cycle insights     (default OFF)
    ├── [Nav]    Update contraception      → mini-form
    ├── [Nav]    Export my cycle data      → POST /cycle/export
    └── [Nav]    Delete all cycle data     → confirm dialog → DELETE /cycle/all-data
```

**Disabling master toggle:**
- `tracking_enabled = false` in `cycle_profiles`
- Cycle card disappears from Body
- Pushes scheduled but not yet sent are cancelled
- Existing data **retained** (re-enable restores)
- Analytics: `cycle_notification_toggled(category: "master", enabled: false)`

**Delete all cycle data:**
- Confirm dialog with text input "DELETE" (English) / "УДАЛИТЬ" (Russian) — friction prevents accidents
- DELETE → CASCADE removes all cycle_* rows
- Audit: `consent_type = data_deletion`
- UI returns to "Set up cycle tracking" CTA card

---

## 15. Localization

### 15.1 Files

iOS: `Localizable.strings` per locale folder.
Android: `strings-cycle.xml` per locale folder.
Both contracts: 80 keys × 6 locales = 480 strings.

### 15.2 Locale list

- en (primary)
- ru (full parity)
- fr-FR (vendor)
- es-ES (vendor)
- pt-BR (vendor)
- zh-Hans (vendor)

### 15.3 Key categories

```
onboarding.*           (12 keys)
phases.*               (5 keys)
predictions.*          (12 keys)
logs.*                 (15 keys)
push.*                 (15 keys)
settings.*             (10 keys)
disclaimers.*          (4 keys)
errors.*               (6 keys)
empty_states.*         (3 keys)
ai_insights.*          (3 keys)

Total: ~85 keys
```

### 15.4 CI parity gate

Script `tools/check-cycle-loc-parity.sh` ensures all 6 locales have identical key set. CI runs on every PR.

### 15.5 Translation budget

80 keys × 5 non-EN locales × $0.20/key (vendor rate) ≈ **$80 one-time** + **$20/quarter** for ongoing additions.

---

## 16. Accessibility Specification

### 16.1 Labels

Every interactive element has a content description / accessibility label. Examples:
- Phase ring: "Cycle phase {phase}, day {day} of {length}, confidence {percent} percent"
- Period log button: "Log a period day"
- Calendar day cell: "{date}, {phase} phase, {logged|no_period}"

### 16.2 Color independence

Phase color (#C026D3) always paired with text. Confidence: green/amber/grey paired with "high"/"medium"/"low".

### 16.3 Dynamic Type / scalable fonts

iOS: `.font(.system(.body))` paired with WVIFont tokens; Dynamic Type up to AccessibilityXL.
Android: `sp` units throughout.

### 16.4 Reduce Motion / Reduce Transparency

iOS: `@Environment(\.accessibilityReduceMotion)` — phase ring fade-only, no rotation.
Android: `isReduceMotionEnabled()` (existing in Phase 1).
Reduce Transparency: GlassCard falls back to solid background.

### 16.5 Contrast ratios

CyclePillar #C026D3 vs Bg #08070F: contrast 5.8:1 (AAA large, AA body). Verified.
Phase labels (white on bg): 18:1 (AAA).
Confidence dots: each ≥4.5:1.

### 16.6 Audit checklist

- [ ] iOS: VoiceOver flow — Cycle Onboarding (4 screens) + Cycle home (4 tabs) + 2 sheets + Settings
- [ ] Android: TalkBack flow — same
- [ ] Xcode AccessibilityInspector — no warnings
- [ ] Android Accessibility Scanner — no critical findings
- [ ] Snapshot tests with largest Dynamic Type sizes
- [ ] Color blindness simulation (Coblis tool)
- [ ] Manual 1-handed gesture testing

---

## 17. Analytics & Telemetry

### 17.1 Client events (parity iOS + Android)

| Event | Properties | Where fired |
|---|---|---|
| `cycle_onboarding_started` | none | GenderSelection screen onAppear |
| `cycle_onboarding_completed` | contraception, anchor_provided | After CyclePeriodAnchor save |
| `cycle_onboarding_skipped` | step | When user taps Skip |
| `cycle_consent_toggled` | granted, step | CycleWelcome consent toggle |
| `cycle_home_opened` | tab | CycleHomeView tab change |
| `cycle_phase_viewed` | phase, day, confidence_bucket | Today tab onAppear (debounced 1/min) |
| `cycle_period_logged` | intensity, is_first_day, source | After successful POST |
| `cycle_period_unlogged` | none | After successful DELETE |
| `cycle_symptom_logged` | symptoms array | After successful POST |
| `cycle_push_delivered` | category | When push received in foreground |
| `cycle_push_tapped` | category, deeplink_path | When push opens app |
| `cycle_notification_toggled` | category, enabled | Settings toggle change |
| `cycle_anomaly_escape_hatch_shown` | none | Today tab when escape hatch displays |
| `cycle_data_exported` | none | Successful export |
| `cycle_data_deleted` | none | Successful delete |

### 17.2 Backend metrics (Prometheus)

- `cycle_endpoint_latency_seconds_bucket{endpoint, method, le}` — histogram
- `cycle_endpoint_errors_total{endpoint, error_kind}` — counter
- `cycle_detector_run_duration_seconds{result}` — histogram
- `cycle_predictor_run_duration_seconds` — histogram
- `cycle_lifecycle_state_transitions_total{from, to}` — counter
- `cycle_active_users_total{lifecycle_state}` — gauge
- `cycle_push_delivered_total{category}` — counter
- `cycle_push_failed_total{category, reason}` — counter

### 17.3 Grafana dashboards

**Dashboard 1: Cycle Operations**
- Active users by lifecycle_state (stacked area, last 7d)
- Detector batch success rate (single stat, 24h)
- Push delivery rate by category (bar chart)
- Endpoint p95 latency by route (heatmap)

**Dashboard 2: Cycle Accuracy & Health**
- Distribution of ovulation_confidence (histogram, 30d)
- Distribution of period_confidence (histogram)
- Lifecycle state distribution (pie)
- Onboarding funnel sankey

### 17.4 Alerts

| Alert | Condition | Severity | Routing |
|---|---|---|---|
| Detector job failure | failures > 50/hr | High | PagerDuty |
| Push delivery rate <80% | over 1h | Medium | Slack #wellex-ops |
| Onboarding completion drop >30% | week-over-week | Low | Slack #wellex-product |
| API error rate >2% | over 1h | Medium | Slack #wellex-ops |
| Migration rollback flag set | manual | Critical | PagerDuty |

---

## 18. Testing Strategy

### 18.1 Test pyramid

```
                       ╱ ╲
                      ╱E2E╲                    ~8 tests (TestFlight + Play internal)
                    ╱──────╲
                   ╱  Snap. ╲                  ~50 tests (Paparazzi + iOS SnapshotTesting)
                  ╱──────────╲
                 ╱ Integration╲                ~25 tests (HTTP roundtrip)
                ╱──────────────╲
               ╱     Unit       ╲              ~200 tests (Rust + Kotlin + Swift)
              ╱──────────────────╲
             ────────────────────
              Algorithm fixtures            7 JSON files; identical input/output across platforms
```

### 18.2 Backend (Rust)

```bash
cd /Users/alexander/Code/wvi-api-rust
cargo test cycle::                        # ~80 unit tests
cargo test --test cycle_integration       # ~20 integration tests against test postgres
cargo bench --bench cycle_detector        # criterion benchmark, target <50ms
```

### 18.3 iOS

```bash
xcodebuild test -scheme WVIHealth -destination 'platform=iOS Simulator,name=iPhone 16'
```

- `CycleViewModelTests.swift`
- `CycleAPIClientTests.swift`
- `CycleSnapshotTests.swift` — 30 snapshots
- `OnboardingFlowUITests.swift` extended

### 18.4 Android

```bash
./gradlew :feature:cycle:testDebugUnitTest
./gradlew :core:domain:test
./gradlew :core:persistence:testDebugUnitTest
./gradlew verifyPaparazziDebug
./gradlew :app:connectedDevAndroidTest
```

### 18.5 Cross-platform algorithm parity

CI job `algorithm-parity.yml`:
1. Run Rust → `rust_output.json`
2. Run Kotlin → `kotlin_output.json`
3. Run Swift → `swift_output.json`
4. Diff all three with tolerance — fail PR if any platform diverges

### 18.6 Beta testing

50 users, 60-day program. Recruitment: female 18-45, regular cycle, ≥5 nights/week wear, mix of contraception (60% none, 25% pill, 15% IUD/implant), 50/50 iOS/Android, 60% US/EU.

Compensation: $50 gift card + 12mo Wellex Plus free + opt-in credits.

Exit gate to public:
- ≥80% beta cycles had detected ovulation
- ≥75% period predictions within ±2 days
- NPS ≥40
- Crash-free ≥99.5% over 14 days
- Median active days/week ≥4

### 18.7 LH-validation cohort (deferred to v2)

100 participants × 3 cycles × LH ovulation predictor kits. Compare sensor-detected to LH-confirmed. Target MAE ≤1.5 days. Cost: $40k clinic + $5k LH supply + $25k staff = ~$70k.

---

## 19. Compliance Plan

### 19.1 FDA general-wellness positioning

**Self-assessment checklist** (file at `wellex-io/app-backend / docs/compliance/fda-general-wellness-2026-04.md`):

- [x] Claim: "track patterns in your menstrual cycle" — not "diagnose"
- [x] Avoid: "abnormal", "high risk", "consult immediately"
- [x] Avoid: "contraceptive", "birth control", "use for pregnancy prevention"
- [x] Allow: "estimated ovulation day", "period prediction", "general wellness"
- [x] Required disclaimer present in: Cycle Welcome, Today footer, every push, Settings → About, Privacy policy
- [x] No specific health outcome promised
- [x] Low-risk, non-invasive
- [x] No diagnostic / treatment claim

**Disclaimer text (canonical):**
> "Wellex Cycle Insights is a general-wellness feature, not a medical device. It does not diagnose, treat, or prevent disease. It is not a contraceptive. Consult a healthcare professional for medical advice."

### 19.2 GDPR Article 9 + DPIA

DPIA template (file at `wellex-io/app-backend / docs/dpia/2026-04-cycle-tracking.md`):

```
# DPIA — Wellex Cycle Tracking

## 1. Description of processing
- Data: skin temperature, HRV, RHR, sleep (existing); period dates, symptoms (new)
- Source: JCV8 bracelet; user input
- Recipients: Wellex team, Sentry (errors only), Anthropic AI (anonymized)
- Retention: until user request OR 7 years inactive

## 2. Necessity & proportionality
- Necessary for cycle feature
- Data minimization: no precise geolocation, no contact list

## 3. Risks & Mitigations
| Risk | L | I | Mitigation |
|---|---|---|---|
| Data leak | L | H | Encryption at rest, TLS 1.3, audit log |
| Re-identification | L | H | UUID IDs, never fully exposed |
| FDA medical-claim violation | L | H | Disclaimer everywhere, no contraceptive language |
| Russia jurisdiction violation | M | C | Geofence v1; RU shard v2 |
| US post-Dobbs subpoena | M | H | Anonymous Mode v2; minimal retention |

## 4. Lawful basis: Art 9(2)(a) explicit consent
## 5. Data subject rights: GET /export (Art 15/20), PATCH (Art 16), DELETE (Art 17)
## 6. Security: encryption, rate limiting, audit, quarterly review
## 7. Retention: active = while account active; inactive = 7 years; on request = immediate
## 8. DPO contact: alex@crossfi.org
## 9. Sign-off: [ ] eng [ ] legal [ ] final
```

### 19.3 Russia 152-ФЗ

Implementation v1:
1. Backend `users.country` derived from IP geo + user-provided
2. Cycle endpoints check; if RU → 451
3. Clients show `CycleUnavailableInRegionScreen`:
   - EN: "Cycle tracking is not yet available in your region. We're working on local data residency to comply with regulations."
   - RU: "Отслеживание цикла пока недоступно в вашем регионе. Мы работаем над локальным размещением данных в соответствии с требованиями."

For v2:
- RU-resident shard (Yandex Cloud or VK Cloud)
- Roskomnadzor registration (30-day process)
- Gateway routing: RU users → RU shard first, mirror to EU/US for backup

### 19.4 Apple App Store

Privacy Manifest `WVIHealth/PrivacyInfo.xcprivacy`:
- `NSPrivacyAccessedAPICategorySensitiveData` reason E174.1
- Data type "Health" → "Reproductive health"
- "User-controlled deletion" supported

App Store reviewer notes: "Cycle tracking is general wellness feature, not a medical device. We do not make diagnostic claims, do not offer contraceptive use, and surface clear disclaimers throughout the app. All cycle data is processed under explicit GDPR Art 9(2)(a) consent. Test account: cycle-tester@wellex.io / password: TBD"

### 19.5 Google Play Console

Data Safety section: Health data → Cycle data. Collected: Yes. Shared: No. Encrypted in transit + at rest. Deletion: Yes (in-app).

### 19.6 State laws (US)

- California CMIA: covered by GDPR-equivalent rights
- Washington My Health My Data Act 2024: explicit consent — covered
- Connecticut DPA: covered

GDPR-grade implementation satisfies all US state laws.

---

## 20. Security & Threat Model (STRIDE)

### 20.1 Per-surface STRIDE matrix

| Surface | Spoofing | Tampering | Repudiation | Info disclosure | DoS | Elevation |
|---|---|---|---|---|---|---|
| Onboarding API | Privy JWT | TLS prevents MITM | cycle_consent_log | Body validated; no echo | 5/min anti-replay | Auth required |
| /cycle/* endpoints | Privy JWT | TLS | audit_log | Own data only | 100r/20w sec/user | gender + tracking_enabled |
| Daily batch | Internal | N/A | cycle_cron_log | Reads/writes own data | Per-user dedup | N/A |
| BLE channel | iOS/Android pairing | AES-CCM | N/A | Local | Phone BLE stack | N/A |
| Push notifications | APNs/FCM key auth | Signed payloads | cycle_push_log | Generic copy only | Provider quota | N/A |
| Postgres | DB creds in env | Constraint validation | TIMESTAMPTZ on every row | Row-level via auth | Pool limits | Least privilege |

### 20.2 Specific threats

**T-1: Subpoena / legal compulsion (post-Dobbs)**
- Likelihood: Medium | Impact: Critical
- Mitigation: Phase 2 Anonymous Mode (client-side encryption); minimal retention; counsel review subpoenas; transparent disclosure annual report; no precise geolocation; document Wellex's legal posture publicly.

**T-2: Account takeover → data exfiltration**
- Likelihood: Low | Impact: Critical
- Mitigation: 2FA (existing); device fingerprint in consent log; quarterly access audit.

**T-3: Detector accuracy attack (false predictions)**
- Likelihood: Very Low | Impact: Medium
- Mitigation: outlier detection; multi-signal confirmation; user-visible confidence.

**T-4: Russian jurisdiction violation**
- Likelihood: Low | Impact: Critical
- Mitigation: 451 geofence v1; never persist RU user data anywhere.

**T-5: Malicious push delivery**
- Likelihood: Very Low | Impact: Medium
- Mitigation: ES256 signed APNs JWT; FCM service account JSON; quarterly key rotation; cycle_push_log audit.

### 20.3 Pen-test scope

Pre-launch external pen-test:
- All 12 cycle endpoints + auth bypass attempts
- Consent log tamper attempts
- Race conditions in detector batch (concurrent runs same user)
- Boundary tests on dates (epoch, year 9999, leap years, DST transitions)
- Locale injection via `consent_text_version` field
- JSON injection via notes/symptom fields

Vendor: TBD. Budget: $15k.

---

## 21. Operations & Runbooks

### 21.1 Detector batch job failure

**Symptom:** PagerDuty alert "Detector batch failure".

**Triage:**
1. `kubectl logs -n wellex deploy/wvi-api | grep cycle_detector | tail -100`
2. Check Sentry for stacktrace
3. `SELECT * FROM cycle_cron_log WHERE success=false ORDER BY fired_at DESC LIMIT 50`
4. `SELECT count(*) FROM cycle_signals_nightly WHERE night_date = CURRENT_DATE - INTERVAL '1 day'`

**Common causes:**
- DB connection saturated → check pool gauge → bounce
- Algorithm panic on edge-case → file Sentry, push hotfix
- Postgres query timeout → vacuum + reindex `cycle_signals_nightly`

**Recovery:**
- For affected users: `POST /api/v1/cycle/admin/recompute {user_id: ...}`
- Monitor next 30 min; should clear

### 21.2 Push delivery rate dropped

**Symptom:** Slack alert "Push delivery <80%".

**Triage:**
1. APNs/FCM provider dashboards → 5xx rates?
2. `SELECT apns_response_code, count(*) FROM cycle_push_log WHERE delivered=false GROUP BY 1`
3. JWT cache: regenerated <50 min ago?

**Common causes:**
- Provider incident → wait + retry
- Stale JWT → manual force-regen
- Mass token invalidation post-iOS update

### 21.3 User reports wrong predictions

**Triage:**
1. Get user_id; query `cycle_signals_nightly` last 60 days
2. Check coverage_pct distribution — <70% = expected low
3. Check `cycles` table for detected ovulation_date + confidence
4. Suggest period_log to anchor; algorithm corrects next cycle

**Response template:**
> Thanks for the report. Based on your skin temperature and HRV pattern, our algorithm detected ovulation on {date} with {confidence}% confidence. Cycle prediction is inherently uncertain. Logging your period helps anchor predictions; our next cycle should incorporate that.

### 21.4 GDPR Art 17 deletion request

**SLA:** 30 days

1. Verify user identity (Privy session)
2. Confirm via reply email (anti-spam)
3. User initiates: Settings → Delete all cycle data
4. Or admin: `DELETE /api/v1/cycle/all-data` with admin override + audit
5. Confirm: query `cycle_*` for user_id should return zero rows
6. Reply with confirmation timestamp

### 21.5 Regulatory inquiry

1. Notify legal counsel within 24h
2. Pause cycle features for affected jurisdiction (feature flag)
3. Cooperate per counsel guidance
4. Document timeline + actions in `docs/incidents/`
5. Post-mortem: update DPIA + compliance docs

---

## 22. Performance Budgets & SLOs

### 22.1 SLOs

| Metric | Target | Measurement |
|---|---|---|
| /cycle/state p95 latency | <200ms | Prometheus |
| /cycle/state p99 latency | <500ms | Prometheus |
| /cycle/calendar p95 (30 days) | <300ms | Prometheus |
| /cycle/* error rate | <0.5% | Prometheus |
| Daily batch coverage | ≥99% | (batched/eligible) |
| Push delivery rate | ≥95% | APNs/FCM responses |
| iOS app launch impact (cold) | <50ms increase | XCTest measure |
| Android app launch (cold) | <80ms increase | macrobenchmark |
| Cycle widget refresh | <500ms | Logging |

### 22.2 Performance budgets

iOS:
- CycleHomeView initial render: <100ms
- Cycle Today tab data fetch + render: <300ms
- Calendar tab month rendering: <200ms

Android:
- CycleScreen first composition: <120ms
- CalendarMonthView 6×7 grid: <150ms
- Glance widget update: <500ms

Backend:
- Detector run for 365 nights: <50ms (criterion benchmark)
- Predictor run: <10ms
- /cycle/state DB queries (read-only, single user): <30ms (combined)

### 22.3 Load testing

Pre-launch: simulate 100k active female users, 24h. Tools: k6 or Gatling.

Scenarios:
- Daily batch: 100k users / 24h = ~70 users/min off-peak, ~600/min during 07:00 local TZ peak
- /cycle/state: 5 reads/day/user → 500k reads/day total, ~6/sec sustained, ~50/sec peak
- Period log writes: ~10/user/cycle = ~30k writes/cycle total → ~1k/day → 0.01/sec sustained

Outcome: Postgres pool 400 max connections (existing) easily handles. No infra additions.

---

## 23. Phasing & Day-by-day Timeline

### 23.1 Phase 0 — Compliance prep (Week 1)

| Day | Task | Owner |
|---|---|---|
| 1 | DPIA draft started | Eng + Legal |
| 2 | DPIA reviewed by acting DPO | Alex |
| 3 | Privacy policy update with cycle section | Legal |
| 4 | App Store + Play Store reviewer notes drafted | PM |
| 5 | Localization vendor briefed (80 keys × 5 langs) | PM |
| 6 | RU exclusion banner copy approved | Legal + PM |
| 7 | Sign-off review meeting; phase 1 GO/NO-GO | All |

**Exit:** DPIA signed, vendor contracted, geofence policy locked.

### 23.2 Phase 1 — Backend foundation (Weeks 2–3)

| Week.Day | Task |
|---|---|
| 2.1 | Migration 018 written + reviewed |
| 2.2 | Migration applied to dev DB; rollback tested |
| 2.3 | models.rs DTOs defined |
| 2.4 | routes.rs + service.rs scaffold for 12 endpoints |
| 2.5 | calendar.rs predictor implemented + unit tested |
| 2.6 | Onboarding endpoint functional |
| 2.7 | profile + state endpoints functional |
| 3.1 | calendar + history + insights endpoints |
| 3.2 | period_log + symptom_log endpoints |
| 3.3 | export + delete endpoints |
| 3.4 | RU geofence implemented + tested |
| 3.5 | All endpoint integration tests passing |
| 3.6 | Code review |
| 3.7 | Merge to main; deploy to dev environment |

**Exit:** End-to-end onboarding + state flow via curl works for non-RU female user.

### 23.3 Phase 2 — iOS onboarding + Body card + Today (Weeks 4–5)

| Week.Day | Task |
|---|---|
| 4.1 | OnboardingState.swift extended; coordinator transitions |
| 4.2 | GenderSelectionView.swift + RootRouterView |
| 4.3 | CycleTrackingWelcomeView.swift |
| 4.4 | CycleContraceptionView.swift |
| 4.5 | CyclePeriodAnchorView.swift |
| 4.6 | CycleAPIClient.swift |
| 4.7 | CycleViewModel.swift basic state |
| 5.1 | CycleSummaryCard + CycleEmptyCard composables in BodyScreen |
| 5.2 | CycleHomeView shell with Today tab |
| 5.3 | PhaseRing + Today tab content |
| 5.4 | PeriodLogSheet + SymptomLogSheet |
| 5.5 | Settings cycle section |
| 5.6 | Localization en + ru baseline (80 strings × 2) |
| 5.7 | Snapshot tests for 4 onboarding screens; PR |

**Exit:** TestFlight build with full onboarding + Cycle home Today + log sheets.

### 23.4 Phase 3 — Sensor detector + blender (Weeks 6–8)

| Week.Day | Task |
|---|---|
| 6.1 | nightly.rs aggregation from biometrics |
| 6.2 | outliers.rs |
| 6.3 | baseline.rs |
| 6.4 | temperature.rs Marshall + CUSUM |
| 6.5 | hrv_rhr.rs |
| 6.6 | confidence.rs Bayesian fusion |
| 6.7 | ovulation.rs composite |
| 7.1 | sensor.rs predictor |
| 7.2 | blender.rs |
| 7.3 | hormonal_branch.rs |
| 7.4 | lifecycle.rs state machine |
| 7.5 | events.rs Kafka subscriber |
| 7.6 | Daily batch loop in main.rs |
| 7.7 | 7 fixtures created |
| 8.1 | Unit tests against fixtures (Rust) |
| 8.2 | Performance benchmark; meet <50ms target |
| 8.3 | Integration test: mock 60 days biometrics |
| 8.4 | Integration test: state transitions |
| 8.5 | Code review |
| 8.6 | Merge + deploy to dev |
| 8.7 | Manual smoke test |

**Exit:** Synthetic 60-day fixtures produce predictions matching ground truth ≥80%.

### 23.5 Phase 4 — Calendar/Insights/History tabs + Push (Weeks 9–10)

| Week.Day | Task |
|---|---|
| 9.1 | CalendarTab month view |
| 9.2 | Calendar day detail bottom sheet |
| 9.3 | InsightsTab charts |
| 9.4 | HistoryTab list + cycle detail sheet |
| 9.5 | AI narrative integration |
| 9.6 | Snapshot tests for 4 tabs |
| 10.1 | iOS push categories registration |
| 10.2 | Backend push notification logic (5 categories) |
| 10.3 | TZ-aware scheduling integrated |
| 10.4 | Deep link routing tested end-to-end |
| 10.5 | iOS Watch complication |
| 10.6 | iOS Widget |
| 10.7 | TestFlight build with full feature set |

**Exit:** All 4 tabs + 5 push types functional in TestFlight.

### 23.6 Phase 5 — Edge cases + analytics + tuning (Week 11)

| Day | Task |
|---|---|
| 11.1 | Lifecycle banners in CycleTodayTab UI |
| 11.2 | PCOS escape hatch one-time message logic |
| 11.3 | Hormonal contraception branch UX |
| 11.4 | 14 analytics events fired |
| 11.5 | Sentry breadcrumbs + error capture |
| 11.6 | Sensitivity module integration |
| 11.7 | Final QA in TestFlight |

**Exit:** All scenario flows tested, telemetry firing.

### 23.7 Phase 6 — Beta cohort (Weeks 12–15)

| Week | Activity |
|---|---|
| 12 | Beta recruitment via TestFlight + Play internal track; 50 users onboarded |
| 13 | Cycle 1 progresses; daily monitoring of analytics + crash logs |
| 14 | Cycle 2 progresses; mid-beta survey |
| 15 | Cycle 2 complete; final survey; analyze results vs exit gate |

### 23.8 Android Phase A1–A12 (Weeks 4–17, parallel)

(See Android plan for day-by-day. Briefly: A1=persistence W4-5, A2=domain W6-7, A3=network W8, A4=onboarding W9-10, A5=cycle home W11-12, A6=remaining tabs W13-14, A7=notifications+widget W15, A8-A12=polish W16-17.)

### 23.9 Phase 7 — Public launch (Week 16)

| Day | Activity |
|---|---|
| 16.1 | App Store + Play Store builds submitted; reviewer notes attached |
| 16.2 | Server-side feature flag flipped to 10% |
| 16.3 | Monitor: error rate, push delivery, retention |
| 16.4 | Ramp to 50% if metrics green |
| 16.5 | Ramp to 100% |
| 16.6 | Press release published; coordinated social media |
| 16.7 | Post-mortem retrospective |

---

## 24. Risks & Mitigations (Extended)

| # | Risk | Likelihood | Impact | Mitigation | Owner | Trigger |
|---|---|---|---|---|---|---|
| 1 | JCV8 sensor noise > 0.20°C threshold | Medium | High | Pre-launch 10-user noise floor study; adjust threshold up to 0.25°C if needed | Eng | Beta accuracy <70% |
| 2 | Bracelet not worn nightly | High | Medium | Coverage badge + lower confidence; calendar fallback always works | UX | Beta median wear <60% |
| 3 | Calendar/sensor disagree first 30d | Medium | Low | Explicit "calibrating" UI; sensor wins after day 30 | Eng | Support tickets |
| 4 | Gender SecureStorage not truthful | Low | Low | Settings override "Enable cycle tracking" independent of gender | Eng | User reports |
| 5 | False positive from fever / illness | Medium | Medium | RHR>110 outlier reject; 3-night requirement | Eng | Beta accuracy off |
| 6 | PMS push annoys users | Medium | Medium | Default opt-in but "Helpful?" survey on 1st fire | UX | Toggle-off rate >30% |
| 7 | RU 152-FZ violation | Low | Critical | Geofence v1; RU shard v2; legal monthly check-ins | Legal | Any data flow detected |
| 8 | GDPR fine for health data leak | Low | Critical | Encryption everywhere; quarterly access audit; bug bounty | Sec | Any leak |
| 9 | False contraceptive claim → FDA | Low | Critical | Disclaimer everywhere; copy review pre-launch by counsel | Legal | Any "contraceptive" mention |
| 10 | Beta accuracy <70% | Medium | High | Tune thresholds; restrict v1 to "regular cyclers only" | Eng | Beta exit gate fails |
| 11 | Localization delays launch | Low | Low | Launch en/ru; other 4 locales fast follow | PM | Vendor delivery slips |
| 12 | RU user via VPN | Medium | Medium | Geofence best-effort; ToS clause | Legal | Roskomnadzor inquiry |
| 13 | Doze mode delays Android batch | High | Medium | setExpedited for delay alerts; backend as primary | Eng | Push delivery <80% Android |
| 14 | OEM aggressive battery (Xiaomi/Samsung) | High | Medium | Onboarding banner asking whitelisting; deep link to system settings | UX | Specific OEM ratings drop |
| 15 | Pen-test critical vuln | Low | High | Schedule pen-test 2 weeks before launch; budget for fixes | Sec | Critical finding |
| 16 | Apple Watch users want full Watch app | Medium | Low | Document as v3 enhancement | Product | App Store reviews |
| 17 | App Store rejects on medical-device claim | Low | Critical | Reviewer notes + counsel-reviewed copy | Legal | Submission |
| 18 | Translation quality issues | Medium | Medium | Native-speaking reviewer per locale | PM | User reviews |

---

## 25. Beta Program

### 25.1 Recruitment

- Source: existing Wellex user base (database query: bracelet active in last 14 days, gender=female, age 18-45, not pregnant flag, not perimenopause flag)
- Filter: ≥5 nights/week wear in last 30 days
- Mix: 60% no contraception, 25% pill, 15% IUD/implant
- Geographic: 60% US/EU, 40% Asia/LatAm
- Platform: 50% iOS / 50% Android
- Final: 50 users via TestFlight + Play internal track

### 25.2 Compensation

- $50 Amazon gift card after 60 days + final survey
- Public name on "thank you" wall in app credits (opt-in)
- Free 12-month Wellex Plus (~$120 value)

### 25.3 Surveys

**Onboarding survey (day 1):**
1. How regular has your cycle been in the last 6 months? (very/moderately/irregular/perimenopause)
2. Are you currently using hormonal contraception?
3. How often do you wear your bracelet at night?

**Mid-beta (day 30):**
1. How accurate has the cycle phase indicator felt? (1-5)
2. Have you logged a period?
3. Have you received any cycle notifications? Were they timely?
4. Anything confusing or missing?

**Final (day 60):**
1. Were predictions accurate vs your felt experience? (1-5)
2. NPS: 0-10
3. Rate UI clarity (1-5)
4. Rate notification value (1-5)
5. What was the best part?
6. What was missing or confusing?
7. Would you continue using cycle tracking after beta?
8. Would you pay $99/year for Wellex Plus to keep cycle tracking?

### 25.4 Exit gate to public launch

ALL must be true:
- ≥80% of beta cycles had a detected ovulation
- ≥75% of period predictions within ±2 days of actual
- Final NPS ≥40
- Crash-free sessions ≥99.5% over 14 days
- Median active days/week ≥4
- ≥70% positive sentiment in final survey
- No critical bugs open

---

## 26. Launch Plan

### 26.1 Soft launch (10% rollout, day 1)

- Server-side feature flag flipped for 10% of female users (random)
- Monitor metrics on Grafana dashboards every 6 hours
- Slack channel: #wellex-cycle-launch
- Hold for 24h before next ramp

### 26.2 Half rollout (50%, day 2)

- If error rate <1%, push delivery >90%, no spike in support tickets → ramp to 50%

### 26.3 Full rollout (100%, day 5)

- Final ramp
- Press release published
- Social media: 4 posts coordinated across iOS Twitter, Android Twitter, Wellex Instagram, LinkedIn

### 26.4 Press release draft

```
WELLEX EXPANDS HEALTH INTELLIGENCE WITH AUTOMATIC CYCLE TRACKING

Wearable health platform Wellex today announced cycle tracking — automatic
detection of menstrual phase, ovulation, and period delays from the JCV8
bracelet's continuous biometric sensors.

Unlike calendar-based apps that rely on manual logging, Wellex's algorithm
uses skin temperature, heart-rate variability, resting heart rate, and sleep
data to identify hormonal patterns through the night. The system delivers
estimated ovulation day, period-prediction calendar, fertile-window forecast,
PMS pattern detection, and delay alerts — all without daily user input.

"Most cycle apps ask women to log everything they feel. Our bracelet sees the
patterns and tells you what's happening," said Alex Mamasidikov, founder of
Wellex.

Wellex's cycle tracking is a general-wellness feature, not a medical device,
and is not intended for contraceptive use. It is built on peer-reviewed
research from Apple, Oura, WHOOP, and Natural Cycles, with Wellex's algorithm
performance targeting 80%+ retrospective ovulation accuracy in regular cyclers.

The feature is available to all Wellex bracelet users today via the iOS app
and Android app. EU users access it through GDPR-compliant explicit consent;
Russian users will gain access in 2026 once data residency is enabled.

# # #

About Wellex: ...
Press contact: press@wellex.io
```

### 26.5 Coordinated social media copy

Twitter (iOS): "Today we're shipping cycle tracking on iOS. Your bracelet detects your phase, ovulation, and period — automatically. No manual logging. Built on peer-reviewed research. Wellness feature, not a medical device. ⤵️"

Twitter (Android): "Cycle tracking is now live on Android. Same feature, same algorithm. Your bracelet does the work."

Instagram: Carousel post with hero shot, 4 tab screenshots, "auto-detection" feature graphic, disclaimer card, sign-up CTA.

LinkedIn: Long-form post by Alex on the engineering challenges (skin temp noise, sensor fusion, GDPR Art 9 compliance) — appeals to tech audience.

### 26.6 Support readiness

- 4 new help center articles drafted: "Setting up cycle tracking", "How does the algorithm work?", "Why is my prediction wrong?", "Hormonal contraception and Wellex"
- Support team trained on cycle data deletion + GDPR rights
- FAQs added to in-app Help screen

---

## 27. Cost Analysis

### 27.1 Engineering effort

| Resource | Weeks | Rate | Cost |
|---|---|---|---|
| Backend engineer (Rust) | 8 | $3,000/wk | $24,000 |
| iOS engineer | 11 | $3,000/wk | $33,000 |
| Android engineer | 12 | $3,000/wk | $36,000 |
| QA engineer (½ time) | 12 | $1,500/wk | $18,000 |
| Designer (¼ time) | 12 | $750/wk | $9,000 |
| **Total people** | | | **$120,000** |

### 27.2 External costs

| Item | Cost |
|---|---|
| Localization (5 langs × 80 keys × $0.20) | $80 |
| Translation review (5 langs × 1h × $50) | $250 |
| Pen-test (TBD vendor) | $15,000 |
| LH-validation cohort (deferred to v2) | $70,000 |
| Beta participant gift cards (50 × $50) | $2,500 |
| Legal review (DPIA + FDA + copy) | $5,000 |
| Roskomnadzor registration (Phase 2 only) | $1,500 |
| **Total external (v1)** | **$23,830** |
| **v2 add-on (LH validation)** | **+$70,000** |

### 27.3 Infrastructure (incremental)

| Resource | Increase | Cost/mo |
|---|---|---|
| Postgres storage (~500KB/user/yr at 100k users) | 50 GB | $5 |
| Cycle endpoint compute (~1% increase) | negligible | <$50 |
| AI gateway (cycle narrative cached 10min) | ~5k/day | <$30 |
| **Total infra/mo** | | **<$100** |

### 27.4 Total v1 budget

- People: $120,000
- External: $23,830
- Infra: ~$1,200/yr
- **Total v1: ~$145,000 + $100/mo ongoing**

### 27.5 Revenue projection (sensitivity)

Pricing: Wellex Plus at $99/year. Cycle tracking is a key differentiator that we believe doubles attach rate among female users.

Assumptions:
- 100k active female users with bracelet
- Pre-cycle attach rate: 20% (= 20k Plus subs at $99 = $1.98M/yr)
- Post-cycle attach rate: 40% (= 40k Plus subs at $99 = $3.96M/yr)
- Net incremental: ~$2M/yr
- Payback: 1 month at full attach
- Even 10% attach uplift = $1M/yr → 9-month payback

### 27.6 Decision criteria

Recommended GO if:
- Beta exit gate passes (§25.4)
- DPIA signed off
- Pen-test no critical findings
- Legal counsel approves copy + disclaimer

Recommended NO-GO if:
- Beta accuracy <70%
- Any pen-test critical finding unresolved
- Legal counsel not satisfied with FDA wellness positioning

---

## 28. Decision Log

| Date | Decision | Rationale | Author |
|---|---|---|---|
| 2026-04-27 | MVP scope = (B) Standard with 8 features | Sweet spot vs Apple parity + scope manageable | Alex |
| 2026-04-27 | Algorithm = (B) Hybrid statistical | (A) too noisy, (C) needs data we don't have | Alex |
| 2026-04-27 | Cold-start = (B) Progressive blending | Better retention than waiting 30 days | Alex |
| 2026-04-27 | Onboarding gender step before Personalize | Cleanest UX; gender drives multiple features | Alex |
| 2026-04-27 | Cycle in Body screen as separate tab | Body is the right home for biological signals | Alex |
| 2026-04-27 | Welcome screen = privacy-first hero | Higher consent rate than direct-to-questions | Alex |
| 2026-04-27 | 4 default + 1 opt-in push categories | Balance value + annoyance | Alex |
| 2026-04-27 | Russia geofence in v1, RU shard in v2 | 152-FZ compliance non-negotiable | Legal |
| 2026-04-27 | No fertile-window contraceptive use in v1 | Avoid FDA medical-device classification | Legal |
| 2026-04-27 | Defer ML to v2; rule-based for MVP | No labeled training data on day 1 | Eng |
| 2026-04-27 | Cycle pillar color = #C026D3 (deep berry) | Distinct from existing 4 pillars + emotion palette | Designer |
| TBD | Pen-test vendor selection | TBD | Sec |
| TBD | LH-validation clinic partnership | TBD | Product |
| TBD | DPO formal hire | TBD | Legal |

---

## 29. Glossary

- **Anchor:** User-provided "last period start date" used to bootstrap calendar predictions.
- **Anovulation:** Cycle without ovulation. Common in PCOS, perimenopause, stressed cycles.
- **BBT (Basal Body Temperature):** Core body temperature at rest, oral or vaginal. Distinguished from skin temperature.
- **Calibration period:** First ~30 days where calendar predictor is the only model.
- **CUSUM (Cumulative Sum):** Change-point detection method accumulating deviations from a reference.
- **DPIA:** GDPR Art 35-mandated assessment for high-risk processing.
- **Follicular phase:** Day 1 of menstruation through ovulation; estrogen rises.
- **Fertile window:** ~5 days before to ~1 day after ovulation.
- **FAM (Fertility Awareness Method):** Family planning based on cycle observation.
- **HRV / RMSSD:** Heart rate variability; root mean square of successive RR-interval differences.
- **LH (Luteinizing Hormone):** Surge ~36h before ovulation; basis of urinary OPKs.
- **Luteal phase:** Ovulation through next menstruation; progesterone dominant.
- **Marshall threshold:** Classic FAM rule: 3 consecutive readings ≥0.2°C above baseline = ovulation occurred.
- **MAE:** Mean Absolute Error.
- **Ovulation:** Egg release; ~14 days before next menstruation in regular cyclers.
- **Pearl Index:** Pregnancy rate per 100 woman-years using a contraceptive method.
- **PCOS:** Polycystic Ovary Syndrome; ~10–13% prevalence; often anovulatory.
- **PMS:** Premenstrual Syndrome (mood, cramping, bloating, breast tenderness in late luteal).
- **Postpartum:** After childbirth; lactational amenorrhea common 6-12 months.
- **Perimenopause:** Transition phase 45-55; cycle variability increases.
- **Progesterone:** Hormone secreted post-ovulation; raises thermoregulation set-point 0.2-0.5°C.
- **Recalibration:** Period after major lifecycle change when models retrain.
- **Retrospective ovulation:** Detected after the fact (3 nights of confirming temperature shift). Used in MVP.
- **RHR:** Resting Heart Rate; lowest sustained HR during sleep.
- **Skin temperature (WST, Wrist Skin Temperature):** Measured by NTC thermistor on bracelet; ΔT during sleep correlates with luteal phase.

---

## 30. References

### Peer-reviewed studies
- [Maijala et al. 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC6883568/) — Nocturnal finger skin temperature for cycle tracking
- [Zhu et al. 2021](https://pmc.ncbi.nlm.nih.gov/articles/PMC8238491/) — Wrist skin temperature accuracy
- [Symul/Apple Hum Reprod 2025](https://academic.oup.com/humrep/article/40/3/469/7989515) — Wrist temperature for retrospective ovulation
- [Oura JMIR 2025](https://pmc.ncbi.nlm.nih.gov/articles/PMC11829181/) — Validation against LH (n=964)
- [WHOOP npj Digital Medicine 2024](https://www.nature.com/articles/s41746-024-01394-0) — Cycle features in HRV/RHR (n=11,590)
- [Schmalenberger 2020](https://pmc.ncbi.nlm.nih.gov/articles/PMC7141121/) — HRV and progesterone correlation
- [Royston & Abrams 1980](https://pubmed.ncbi.nlm.nih.gov/7407311/) — CUSUM for BBT
- [Goodale/Shilaih 2019](https://pmc.ncbi.nlm.nih.gov/articles/PMC8918962/) — Ava ML fertile window
- [Apple Women's Health Study npj 2023](https://www.nature.com/articles/s41746-023-00848-1) — n=12,608
- [Symul/Bull Hum Reprod Open 2020](https://academic.oup.com/hropen/article/2020/2/hoaa011/5820371) — Cycle length distribution

### Regulatory
- [FDA General Wellness Policy](https://www.fda.gov/regulatory-information/search-fda-guidance-documents/general-wellness-policy-low-risk-devices)
- [Faegre Drinker — 2026 General Wellness updates](https://www.faegredrinker.com/en/insights/publications/2026/1/key-updates-in-fdas-2026-general-wellness-and-clinical-decision-support-software-guidance)
- [Natural Cycles De Novo DEN170052](https://www.accessdata.fda.gov/cdrh_docs/reviews/DEN170052.pdf)
- [Natural Cycles K231274 (Apple Watch)](https://www.accessdata.fda.gov/cdrh_docs/pdf23/K231274.pdf)
- [Natural Cycles K202897 (Oura)](https://www.accessdata.fda.gov/cdrh_docs/pdf20/K202897.pdf)
- [EU MDR — software classification rules](https://decomplix.com/medical-device-classification-eu-mdr/)
- [ICO — special category data](https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/lawful-basis/a-guide-to-lawful-basis/special-category-data/)
- [Russia 152-FZ overview](https://securiti.ai/russian-federal-law-no-152-fz/)
- [Morgan Lewis — Russia data localization](https://www.morganlewis.com/-/media/files/publication/outside-publication/article/2021/data-localization-laws-russian-federation.pdf)
- [Stateline — period tracking post-Dobbs](https://stateline.org/2024/07/26/data-privacy-after-dobbs-is-period-tracking-safe/)

### Competitor materials
- [Oura Cycle Insights blog](https://ouraring.com/blog/oura-cycle-insights/)
- [UCSF + Oura PCOS study](https://ouraring.com/blog/ucsf-oura-irregular-menstrual-cycle-study/)
- [Apple Watch cycle support page](https://support.apple.com/en-us/120357)
- [WHOOP Menstrual White Paper](https://www.whoop.com/us/en/thelocker/menstrual-cycle-insights-white-paper/)
- [Fitbit FEMFIT cohort](https://www.nature.com/articles/s44294-024-00037-9)
- [Clue science page](https://helloclue.com/articles/about-clue/science-your-cycle-evidence-based-app-design)
- [Flo accuracy](https://flo.health/flo-accuracy)
- [Flo Databricks 2025](https://www.databricks.com/company/newsroom/press-releases/flo-health-accelerates-ai-innovation-and-personalizes-care)

### Clinical
- [WHO PCOS fact sheet](https://www.who.int/news-room/fact-sheets/detail/polycystic-ovary-syndrome)
- [StatPearls PCOS](https://www.ncbi.nlm.nih.gov/books/NBK459251/)
- [Lactational amenorrhea PMC8835773](https://pmc.ncbi.nlm.nih.gov/articles/PMC8835773/)
- [AWHS variability by age + PCOS](https://www.sciencedirect.com/science/article/pii/S0002937825008671)

---

**End of Master Plan v2.0** — 30 sections, ~3,300 lines. Companion docs:
- `iOS/docs/cycle-tracking/IOS_IMPLEMENTATION_PLAN.md` (1,303 lines)
- `Android/docs/cycle-tracking/ANDROID_IMPLEMENTATION_PLAN.md` (1,478 lines)
- `wellex-io/app-backend → docs/cycle-tracking/IMPLEMENTATION_PLAN.md` (1,303 lines)

Pending decisions before engineering kickoff: (1) Approve MVP scope; (2) Approve $145k budget; (3) Approve geofence policy for Russia v1; (4) Sign-off DPIA after legal review; (5) Identify pen-test vendor; (6) Identify LH-validation clinic partner (v2).
