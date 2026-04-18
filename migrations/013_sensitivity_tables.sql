-- 013_sensitivity_tables.sql — Project B — sensitivity pipeline tables.
--
-- Per-context personal baselines, ensemble change-point signals, composite
-- multi-metric events, AI insight cache, and rolling detector state for the
-- CUSUM/EWMA recurrences.

CREATE TABLE user_baselines (
  user_id UUID NOT NULL,
  metric_type TEXT NOT NULL,
  context_key TEXT NOT NULL,
  mean DOUBLE PRECISION NOT NULL,
  std DOUBLE PRECISION NOT NULL,
  p10 DOUBLE PRECISION NOT NULL,
  p90 DOUBLE PRECISION NOT NULL,
  sample_count INT NOT NULL DEFAULT 0,
  locked BOOLEAN NOT NULL DEFAULT false,
  last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, metric_type, context_key)
);

CREATE TABLE signals (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  metric_type TEXT NOT NULL,
  context_key TEXT NOT NULL,
  deviation_sigma DOUBLE PRECISION NOT NULL,
  direction TEXT NOT NULL,
  severity TEXT NOT NULL,
  detectors_fired JSONB NOT NULL,
  bayesian_confidence DOUBLE PRECISION,
  rarity_percentile DOUBLE PRECISION,
  narrative TEXT,
  ack BOOLEAN NOT NULL DEFAULT false,
  CONSTRAINT valid_direction CHECK (direction IN ('up','down')),
  CONSTRAINT valid_severity CHECK (severity IN ('low','medium','high'))
);
CREATE INDEX idx_signals_user_ts ON signals (user_id, ts DESC);

CREATE TABLE composite_signals (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  pair_id TEXT NOT NULL,
  component_signal_ids UUID[] NOT NULL,
  anomaly_percentile DOUBLE PRECISION NOT NULL,
  narrative TEXT,
  severity TEXT NOT NULL
);
CREATE INDEX idx_composite_user_ts ON composite_signals (user_id, ts DESC);

CREATE TABLE ai_insights_cache (
  user_id UUID NOT NULL,
  screen_key TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  content TEXT NOT NULL,
  generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, screen_key)
);

CREATE TABLE correlation_pairs_config (
  pair_id TEXT PRIMARY KEY,
  metric_a TEXT NOT NULL,
  direction_a TEXT NOT NULL,
  metric_b TEXT NOT NULL,
  direction_b TEXT NOT NULL,
  severity_boost TEXT NOT NULL,
  description TEXT NOT NULL
);

INSERT INTO correlation_pairs_config VALUES
  ('hrv_down_stress_up',          'hrv', 'down', 'stress', 'up', 'high', 'Sympathetic activation'),
  ('hr_up_rest_hrv_down',         'hr', 'up',   'hrv', 'down', 'high', 'Illness or overtraining indicator'),
  ('temp_up_energy_down',         'temp', 'up', 'energy', 'down', 'medium', 'Possible illness onset'),
  ('coherence_down_emotion_neg',  'coherence', 'down', 'emotion_confidence', 'down', 'medium', 'Emotional stress'),
  ('sleep_debt_up_recovery_down', 'sleep', 'down', 'recovery', 'down', 'high', 'Chronic sleep deficit'),
  ('activity_up_hrv_next_day',    'activity_intensity', 'up', 'hrv', 'down', 'medium', 'Overtraining'),
  ('spo2_night_sleep_quality',    'spo2', 'down', 'sleep', 'down', 'high', 'Respiratory concern'),
  ('breathing_rate_stress_up',    'breathing_rate', 'up', 'stress', 'up', 'medium', 'Anxiety episode'),
  ('wvi_trend_down_3d',           'wvi', 'down', 'wvi', 'down', 'high', 'Systemic decline'),
  ('emotion_focus_coherence_up',  'emotion_confidence', 'up', 'coherence', 'up', 'low', 'Flow state (positive)');

CREATE TABLE detector_state (
  user_id UUID NOT NULL,
  metric_type TEXT NOT NULL,
  state JSONB NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (user_id, metric_type)
);
