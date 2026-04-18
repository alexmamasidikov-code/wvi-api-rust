-- Track provenance of every BP reading so we can prioritise manual+Apple Health
-- over the estimated fallback used when no real measurement is available.
ALTER TABLE hrv ADD COLUMN IF NOT EXISTS bp_source TEXT NOT NULL DEFAULT 'estimated';

-- Partial index over the "real" rows (manual + healthkit). Makes the priority
-- lookup in GET /biometrics/blood-pressure fast and keeps the index small
-- (estimated rows are never written — only read-from-estimation at GET time).
CREATE INDEX IF NOT EXISTS idx_hrv_user_bp_source
    ON hrv (user_id, bp_source, timestamp DESC)
    WHERE bp_source IN ('manual', 'healthkit');

-- 4h crisis-push dedup log. Small, append-only. One row per push sent.
CREATE TABLE IF NOT EXISTS push_notifications_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    category TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_push_notif_log_user_cat_ts
    ON push_notifications_log (user_id, category, sent_at DESC);
