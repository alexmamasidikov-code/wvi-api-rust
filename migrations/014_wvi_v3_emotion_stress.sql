-- 014_wvi_v3_emotion_stress.sql
-- Project C — WVI v3 + Emotion/Stress Mega-Sensitivity tables.

-- ═══════════════════════════════════════════════════════════════════════════
-- WVI v3 user profile + display mode + weights
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS user_wvi_profile (
  user_id UUID PRIMARY KEY,
  profile TEXT NOT NULL DEFAULT 'balanced',
  auto_classified_profile TEXT,
  classification_updated_at TIMESTAMPTZ,
  display_mode TEXT NOT NULL DEFAULT 'rich',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT valid_profile CHECK (profile IN ('athlete','office','recovery','stressed','senior','balanced','auto')),
  CONSTRAINT valid_display CHECK (display_mode IN ('rich','simple'))
);

CREATE TABLE IF NOT EXISTS profile_component_weights (
  profile TEXT NOT NULL,
  component TEXT NOT NULL,
  weight DOUBLE PRECISION NOT NULL,
  PRIMARY KEY (profile, component)
);

-- Seed weights for 6 profiles × 18 components.
-- Components: hrv_personal, signal_burden, recovery_momentum, sleep_composite, circadian_alignment,
--             activity_personal, metabolic_efficiency, stress_personal, breathing_rate_rest, coherence_personal,
--             immune_proxy, intraday_stability, emotion_agility, emotion_range, emotion_anchors,
--             emotion_regulation, emotion_diversity, emotion_contagion
INSERT INTO profile_component_weights VALUES
  -- Balanced: uniform ~1/18
  ('balanced','hrv_personal',0.055), ('balanced','signal_burden',0.055), ('balanced','recovery_momentum',0.055),
  ('balanced','sleep_composite',0.055), ('balanced','circadian_alignment',0.055),
  ('balanced','activity_personal',0.055), ('balanced','metabolic_efficiency',0.055),
  ('balanced','stress_personal',0.055), ('balanced','breathing_rate_rest',0.055), ('balanced','coherence_personal',0.055),
  ('balanced','immune_proxy',0.055), ('balanced','intraday_stability',0.055),
  ('balanced','emotion_agility',0.055), ('balanced','emotion_range',0.055), ('balanced','emotion_anchors',0.055),
  ('balanced','emotion_regulation',0.055), ('balanced','emotion_diversity',0.055), ('balanced','emotion_contagion',0.06),

  -- Athlete: activity + recovery emphasized
  ('athlete','hrv_personal',0.08), ('athlete','signal_burden',0.04), ('athlete','recovery_momentum',0.10),
  ('athlete','sleep_composite',0.08), ('athlete','circadian_alignment',0.04),
  ('athlete','activity_personal',0.12), ('athlete','metabolic_efficiency',0.10),
  ('athlete','stress_personal',0.04), ('athlete','breathing_rate_rest',0.03), ('athlete','coherence_personal',0.04),
  ('athlete','immune_proxy',0.05), ('athlete','intraday_stability',0.04),
  ('athlete','emotion_agility',0.04), ('athlete','emotion_range',0.04), ('athlete','emotion_anchors',0.04),
  ('athlete','emotion_regulation',0.04), ('athlete','emotion_diversity',0.04), ('athlete','emotion_contagion',0.04),

  -- Office: sleep + stress emphasized
  ('office','hrv_personal',0.05), ('office','signal_burden',0.08), ('office','recovery_momentum',0.04),
  ('office','sleep_composite',0.10), ('office','circadian_alignment',0.06),
  ('office','activity_personal',0.04), ('office','metabolic_efficiency',0.03),
  ('office','stress_personal',0.10), ('office','breathing_rate_rest',0.06), ('office','coherence_personal',0.06),
  ('office','immune_proxy',0.05), ('office','intraday_stability',0.05),
  ('office','emotion_agility',0.05), ('office','emotion_range',0.04), ('office','emotion_anchors',0.04),
  ('office','emotion_regulation',0.05), ('office','emotion_diversity',0.04), ('office','emotion_contagion',0.06),

  -- Recovery
  ('recovery','hrv_personal',0.08), ('recovery','signal_burden',0.06), ('recovery','recovery_momentum',0.12),
  ('recovery','sleep_composite',0.12), ('recovery','circadian_alignment',0.06),
  ('recovery','activity_personal',0.03), ('recovery','metabolic_efficiency',0.03),
  ('recovery','stress_personal',0.06), ('recovery','breathing_rate_rest',0.04), ('recovery','coherence_personal',0.05),
  ('recovery','immune_proxy',0.10), ('recovery','intraday_stability',0.05),
  ('recovery','emotion_agility',0.04), ('recovery','emotion_range',0.03), ('recovery','emotion_anchors',0.03),
  ('recovery','emotion_regulation',0.04), ('recovery','emotion_diversity',0.03), ('recovery','emotion_contagion',0.03),

  -- Stressed
  ('stressed','hrv_personal',0.06), ('stressed','signal_burden',0.08), ('stressed','recovery_momentum',0.04),
  ('stressed','sleep_composite',0.06), ('stressed','circadian_alignment',0.04),
  ('stressed','activity_personal',0.04), ('stressed','metabolic_efficiency',0.03),
  ('stressed','stress_personal',0.12), ('stressed','breathing_rate_rest',0.08), ('stressed','coherence_personal',0.10),
  ('stressed','immune_proxy',0.04), ('stressed','intraday_stability',0.06),
  ('stressed','emotion_agility',0.05), ('stressed','emotion_range',0.04), ('stressed','emotion_anchors',0.03),
  ('stressed','emotion_regulation',0.06), ('stressed','emotion_diversity',0.03), ('stressed','emotion_contagion',0.04),

  -- Senior
  ('senior','hrv_personal',0.05), ('senior','signal_burden',0.06), ('senior','recovery_momentum',0.04),
  ('senior','sleep_composite',0.12), ('senior','circadian_alignment',0.10),
  ('senior','activity_personal',0.04), ('senior','metabolic_efficiency',0.03),
  ('senior','stress_personal',0.05), ('senior','breathing_rate_rest',0.05), ('senior','coherence_personal',0.05),
  ('senior','immune_proxy',0.10), ('senior','intraday_stability',0.06),
  ('senior','emotion_agility',0.04), ('senior','emotion_range',0.04), ('senior','emotion_anchors',0.04),
  ('senior','emotion_regulation',0.05), ('senior','emotion_diversity',0.04), ('senior','emotion_contagion',0.04)
ON CONFLICT (profile, component) DO NOTHING;

CREATE TABLE IF NOT EXISTS circadian_baselines (
  user_id UUID NOT NULL,
  metric_type TEXT NOT NULL,
  hour_of_day INT NOT NULL,
  mean DOUBLE PRECISION NOT NULL,
  std DOUBLE PRECISION NOT NULL,
  sample_count INT NOT NULL,
  last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, metric_type, hour_of_day)
);

CREATE TABLE IF NOT EXISTS wvi_forecast_cache (
  user_id UUID PRIMARY KEY,
  generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  horizon_6h JSONB NOT NULL,
  horizon_24h JSONB NOT NULL,
  timeline JSONB NOT NULL,
  narrative TEXT
);

CREATE TABLE IF NOT EXISTS wvi_ai_reweight_cache (
  user_id UUID PRIMARY KEY,
  generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  weight_deltas JSONB NOT NULL,
  rationale TEXT,
  uses_today INT NOT NULL DEFAULT 0
);

-- ═══════════════════════════════════════════════════════════════════════════
-- Emotion tables (v2)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS emotion_samples_1min (
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  valence DOUBLE PRECISION NOT NULL,
  arousal DOUBLE PRECISION NOT NULL,
  confidence DOUBLE PRECISION NOT NULL,
  primary_emotion TEXT NOT NULL,
  primary_intensity DOUBLE PRECISION NOT NULL,
  secondary_emotion TEXT NOT NULL,
  secondary_intensity DOUBLE PRECISION NOT NULL,
  tertiary_emotion TEXT NOT NULL,
  tertiary_intensity DOUBLE PRECISION NOT NULL,
  context_key TEXT NOT NULL,
  PRIMARY KEY (user_id, ts)
);

CREATE INDEX IF NOT EXISTS idx_emotion_1min_user_ts ON emotion_samples_1min (user_id, ts DESC);

CREATE TABLE IF NOT EXISTS emotion_samples_5min (
  user_id UUID NOT NULL,
  bucket_ts TIMESTAMPTZ NOT NULL,
  valence_mean DOUBLE PRECISION NOT NULL,
  arousal_mean DOUBLE PRECISION NOT NULL,
  primary_mode TEXT NOT NULL,
  dwell_seconds_per_region JSONB NOT NULL,
  PRIMARY KEY (user_id, bucket_ts)
);

CREATE TABLE IF NOT EXISTS user_emotion_baselines (
  user_id UUID NOT NULL,
  context_key TEXT NOT NULL,
  v_mean DOUBLE PRECISION NOT NULL,
  a_mean DOUBLE PRECISION NOT NULL,
  v_std DOUBLE PRECISION NOT NULL,
  a_std DOUBLE PRECISION NOT NULL,
  anchors JSONB NOT NULL DEFAULT '[]',
  frequency_distribution JSONB NOT NULL DEFAULT '{}',
  locked BOOLEAN NOT NULL DEFAULT false,
  last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, context_key)
);

CREATE TABLE IF NOT EXISTS emotion_triggers (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL,
  shift_ts TIMESTAMPTZ NOT NULL,
  event_id UUID NOT NULL,
  correlation_p DOUBLE PRECISION NOT NULL,
  shift_from_region TEXT NOT NULL,
  shift_to_region TEXT NOT NULL,
  magnitude DOUBLE PRECISION NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_emotion_triggers_user_ts ON emotion_triggers (user_id, shift_ts DESC);

CREATE TABLE IF NOT EXISTS emotion_ai_narratives (
  user_id UUID NOT NULL,
  narrative_type TEXT NOT NULL,
  content TEXT NOT NULL,
  generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, narrative_type)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- Stress tables (v2)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS stress_samples_1min (
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  score DOUBLE PRECISION NOT NULL,
  level TEXT NOT NULL,
  micro_pulse BOOLEAN NOT NULL DEFAULT false,
  sympathetic_proxy DOUBLE PRECISION,
  parasympathetic_proxy DOUBLE PRECISION,
  baseline_delta DOUBLE PRECISION,
  PRIMARY KEY (user_id, ts)
);

CREATE INDEX IF NOT EXISTS idx_stress_1min_user_ts ON stress_samples_1min (user_id, ts DESC);

CREATE TABLE IF NOT EXISTS stress_ai_narratives (
  user_id UUID NOT NULL,
  narrative_type TEXT NOT NULL,
  content TEXT NOT NULL,
  generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, narrative_type)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- Formula version bumps (conditional — only if formula_versions exists)
-- ═══════════════════════════════════════════════════════════════════════════

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'formula_versions') THEN
    INSERT INTO formula_versions (metric_type, version, valid_from)
    VALUES ('wvi', 3, NOW()),
           ('valence', 2, NOW()),
           ('arousal', 2, NOW()),
           ('stress', 2, NOW())
    ON CONFLICT DO NOTHING;
  END IF;
END $$;
