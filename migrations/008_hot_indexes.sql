-- Hot query indexes: (user_id, timestamp DESC) for biometric read paths.
-- Idempotent: IF NOT EXISTS covers indexes already declared in 001_initial.sql.
CREATE INDEX IF NOT EXISTS idx_hr_user_ts ON heart_rate(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_hrv_user_ts ON hrv(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_spo2_user_ts ON spo2(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_temp_user_ts ON temperature(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_activity_user_ts ON activity(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_ppi_user_ts ON ppi(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_ecg_user_ts ON ecg(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_sleep_user_date ON sleep_records(user_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_wvi_user_ts ON wvi_scores(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emotions_user_ts ON emotions(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_alerts_user_ts ON alerts(user_id, created_at DESC);
