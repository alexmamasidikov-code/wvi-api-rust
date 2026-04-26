#!/usr/bin/env bash
# Seeds the Apple App Store reviewer account (apple-review@wellex.io)
# with 90 days of realistic biometric data so the reviewer can exercise
# the full app surface without a JCV8 bracelet.
#
# Run on the database server (or any host with psql + DATABASE_URL set):
#   DATABASE_URL=postgres://wvi:wvi_dev_2026@postgres:5432/wvi \
#     ./scripts/seed_reviewer.sh

set -e

: "${DATABASE_URL:?Set DATABASE_URL=postgres://… before running}"
REVIEWER_EMAIL="${REVIEWER_EMAIL:-apple-review@wellex.io}"
REVIEWER_DID="${REVIEWER_DID:-did:privy:apple-review-001}"

echo "→ seeding reviewer account ${REVIEWER_EMAIL}"

psql "$DATABASE_URL" -v ON_ERROR_STOP=1 <<SQL
-- 1. upsert the reviewer user with is_reviewer = TRUE
INSERT INTO users (id, privy_did, email, name, password_hash, is_reviewer, created_at, updated_at)
VALUES (
    gen_random_uuid(),
    '${REVIEWER_DID}',
    '${REVIEWER_EMAIL}',
    'Apple Reviewer',
    '\$2b\$reviewer\$placeholder\$hash',
    TRUE,
    NOW(),
    NOW()
)
ON CONFLICT (privy_did) DO UPDATE
SET is_reviewer = TRUE,
    email = EXCLUDED.email,
    updated_at = NOW();

-- pick up the resolved user id
WITH reviewer AS (SELECT id FROM users WHERE privy_did = '${REVIEWER_DID}')
SELECT id AS reviewer_id FROM reviewer \gset

-- 2. wipe any prior seed for this reviewer (idempotent)
DELETE FROM heart_rate    WHERE user_id = :'reviewer_id';
DELETE FROM hrv           WHERE user_id = :'reviewer_id';
DELETE FROM spo2          WHERE user_id = :'reviewer_id';
DELETE FROM temperature   WHERE user_id = :'reviewer_id';
DELETE FROM sleep_records WHERE user_id = :'reviewer_id';
DELETE FROM activity      WHERE user_id = :'reviewer_id';
DELETE FROM wvi_scores    WHERE user_id = :'reviewer_id';

-- 3. heart rate — 90 days × 4 samples/day, realistic 60-90 bpm with diurnal swing
INSERT INTO heart_rate (user_id, timestamp, bpm, confidence, zone)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval - (h || ' hours')::interval,
    65 + 12 * SIN(EXTRACT(EPOCH FROM (NOW() - (d || ' days')::interval - (h || ' hours')::interval)) / 14400.0)
       + 8 * RANDOM(),
    0.92 + 0.06 * RANDOM(),
    1 + (RANDOM() * 3)::int
FROM generate_series(0, 89) d, generate_series(0, 23, 6) h;

-- 4. HRV — daily summary, 35-65 ms with mild positive trend
INSERT INTO hrv (user_id, timestamp, rmssd, sdnn, confidence)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval,
    42 + (90 - d) * 0.10 + 6 * RANDOM(),
    52 + (90 - d) * 0.08 + 7 * RANDOM(),
    0.90 + 0.08 * RANDOM()
FROM generate_series(0, 89) d;

-- 5. SpO2 — daily 96-99 %
INSERT INTO spo2 (user_id, timestamp, percent, confidence)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval,
    96.5 + 2.0 * RANDOM(),
    0.93 + 0.05 * RANDOM()
FROM generate_series(0, 89) d;

-- 6. temperature — daily 36.4-36.8 °C
INSERT INTO temperature (user_id, timestamp, celsius, confidence)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval,
    36.45 + 0.35 * RANDOM(),
    0.95
FROM generate_series(0, 89) d;

-- 7. sleep — 90 nightly records, 6-9 h
INSERT INTO sleep_records (user_id, date, bedtime, wake_time, total_hours, sleep_score, efficiency)
SELECT
    :'reviewer_id'::uuid,
    (NOW() - (d || ' days')::interval)::date,
    (NOW() - (d || ' days')::interval)::date - INTERVAL '4 hours',
    (NOW() - (d || ' days')::interval)::date + INTERVAL '4 hours',
    6.5 + 2.0 * RANDOM(),
    65 + 25 * RANDOM(),
    0.78 + 0.18 * RANDOM()
FROM generate_series(0, 89) d;

-- 8. activity — 90 daily records, 4-12k steps
INSERT INTO activity (user_id, timestamp, steps, calories, distance_km)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval,
    4000 + (8000 * RANDOM())::int,
    200 + (350 * RANDOM())::int,
    3.0 + 6.0 * RANDOM()
FROM generate_series(0, 89) d;

-- 9. WVI scores — 90 daily, drift 60→85 with noise
INSERT INTO wvi_scores (user_id, timestamp, wvi_score, level, metrics, weights, emotion_feedback)
SELECT
    :'reviewer_id'::uuid,
    NOW() - (d || ' days')::interval,
    60 + (90 - d) * 0.25 + 5 * RANDOM(),
    CASE
        WHEN 60 + (90 - d) * 0.25 < 65 THEN 'low'
        WHEN 60 + (90 - d) * 0.25 < 80 THEN 'good'
        ELSE 'excellent'
    END,
    '{"hr": 0.8, "hrv": 0.75, "spo2": 0.95, "sleep": 0.82, "stress": 0.40}'::jsonb,
    '{"hr": 1.0, "hrv": 1.0, "spo2": 0.5, "sleep": 1.5, "stress": 1.2}'::jsonb,
    1.0
FROM generate_series(0, 89) d;

-- summary
SELECT
    'heart_rate'  AS table_name, COUNT(*) AS rows FROM heart_rate    WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'hrv',           COUNT(*) FROM hrv             WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'spo2',          COUNT(*) FROM spo2            WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'temperature',   COUNT(*) FROM temperature     WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'sleep_records', COUNT(*) FROM sleep_records   WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'activity',      COUNT(*) FROM activity        WHERE user_id = :'reviewer_id'
UNION ALL SELECT 'wvi_scores',    COUNT(*) FROM wvi_scores      WHERE user_id = :'reviewer_id';
SQL

echo "✓ reviewer account ${REVIEWER_EMAIL} seeded with 90 days of biometrics"
