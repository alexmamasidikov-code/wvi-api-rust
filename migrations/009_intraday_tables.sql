-- 009_intraday_tables.sql — Project A foundation

-- 1-min hot table, partitioned by hour, auto-drop >24h
CREATE TABLE biometrics_1min (
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  metric_type TEXT NOT NULL,
  value DOUBLE PRECISION NOT NULL,
  formula_version INT NOT NULL DEFAULT 1,
  PRIMARY KEY (user_id, ts, metric_type)
) PARTITION BY RANGE (ts);

CREATE INDEX idx_1min_user_metric_ts
  ON biometrics_1min (user_id, metric_type, ts DESC);

-- Create initial partitions for current hour + next 24h (25 hours)
DO $$
DECLARE h TIMESTAMPTZ := date_trunc('hour', NOW());
BEGIN
  FOR i IN 0..24 LOOP
    EXECUTE format(
      'CREATE TABLE IF NOT EXISTS biometrics_1min_%s PARTITION OF biometrics_1min FOR VALUES FROM (%L) TO (%L)',
      to_char(h + (i || ' hours')::interval, 'YYYYMMDDHH24'),
      h + (i || ' hours')::interval,
      h + ((i+1) || ' hours')::interval
    );
  END LOOP;
END $$;

-- 5-min historical, partitioned monthly, auto-drop >365d
CREATE TABLE biometrics_5min (
  user_id UUID NOT NULL,
  bucket_ts TIMESTAMPTZ NOT NULL,
  metric_type TEXT NOT NULL,
  value_mean DOUBLE PRECISION NOT NULL,
  value_min DOUBLE PRECISION,
  value_max DOUBLE PRECISION,
  sample_count INT NOT NULL,
  formula_version INT NOT NULL,
  PRIMARY KEY (user_id, bucket_ts, metric_type)
) PARTITION BY RANGE (bucket_ts);

CREATE INDEX idx_5min_user_metric_bucket
  ON biometrics_5min (user_id, metric_type, bucket_ts DESC);

-- Create current + next 2 months partitions
DO $$
DECLARE m TIMESTAMPTZ := date_trunc('month', NOW());
BEGIN
  FOR i IN 0..2 LOOP
    EXECUTE format(
      'CREATE TABLE IF NOT EXISTS biometrics_5min_%s PARTITION OF biometrics_5min FOR VALUES FROM (%L) TO (%L)',
      to_char(m + (i || ' months')::interval, 'YYYYMM'),
      m + (i || ' months')::interval,
      m + ((i+1) || ' months')::interval
    );
  END LOOP;
END $$;

-- Daily forever
CREATE TABLE biometrics_daily (
  user_id UUID NOT NULL,
  day DATE NOT NULL,
  metric_type TEXT NOT NULL,
  value_mean DOUBLE PRECISION NOT NULL,
  value_min DOUBLE PRECISION,
  value_max DOUBLE PRECISION,
  value_p10 DOUBLE PRECISION,
  value_p90 DOUBLE PRECISION,
  volatility_sd DOUBLE PRECISION,
  PRIMARY KEY (user_id, day, metric_type)
);

CREATE INDEX idx_daily_user_metric_day
  ON biometrics_daily (user_id, metric_type, day DESC);

-- Event annotations
CREATE TABLE intraday_events (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  event_type TEXT NOT NULL,
  meta JSONB,
  CONSTRAINT valid_event_type CHECK (event_type IN
    ('workout','meal','sleep_phase','ai_alert','reminder_fired','crisis','manual_note'))
);

CREATE INDEX idx_events_user_ts ON intraday_events (user_id, ts DESC);

-- Formula versions (CDC tracking)
CREATE TABLE formula_versions (
  metric_type TEXT NOT NULL,
  version INT NOT NULL,
  deployed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  valid_from TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (metric_type, version)
);

-- Seed default v1 versions for all metrics
INSERT INTO formula_versions (metric_type, version, valid_from) VALUES
  ('hr', 1, '2020-01-01'::TIMESTAMPTZ),
  ('hrv', 1, '2020-01-01'::TIMESTAMPTZ),
  ('spo2', 1, '2020-01-01'::TIMESTAMPTZ),
  ('temp', 1, '2020-01-01'::TIMESTAMPTZ),
  ('wvi', 1, '2020-01-01'::TIMESTAMPTZ),
  ('stress', 1, '2020-01-01'::TIMESTAMPTZ),
  ('emotion_confidence', 1, '2020-01-01'::TIMESTAMPTZ),
  ('energy', 1, '2020-01-01'::TIMESTAMPTZ),
  ('recovery', 1, '2020-01-01'::TIMESTAMPTZ),
  ('coherence', 1, '2020-01-01'::TIMESTAMPTZ),
  ('breathing_rate', 1, '2020-01-01'::TIMESTAMPTZ),
  ('activity_intensity', 1, '2020-01-01'::TIMESTAMPTZ);

-- Backfill jobs
CREATE TABLE backfill_jobs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  metric_type TEXT NOT NULL,
  new_version INT NOT NULL,
  started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  completed_at TIMESTAMPTZ,
  progress_ratio DOUBLE PRECISION NOT NULL DEFAULT 0.0,
  last_user_id UUID,
  status TEXT NOT NULL DEFAULT 'running',
  CONSTRAINT valid_status CHECK (status IN ('running','paused','completed','failed'))
);

CREATE INDEX idx_backfill_status ON backfill_jobs (status, started_at DESC);
