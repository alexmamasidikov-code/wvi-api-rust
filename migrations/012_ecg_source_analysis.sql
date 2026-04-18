-- ECG Rework (Project F) — track ECG provenance (bracelet vs Apple Watch)
-- and persist structured AI analysis JSON on the same row so GET listings
-- can serve enriched history without a join.
--
-- No unique (user_id, timestamp) constraint is added: the Project D finding
-- (BP table) showed that adding one mid-stream is risky with fuzzy client
-- timestamps. Callers use ORDER BY + LIMIT instead.

ALTER TABLE ecg ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'bracelet';
ALTER TABLE ecg ADD COLUMN IF NOT EXISTS analysis_json JSONB;

CREATE INDEX IF NOT EXISTS idx_ecg_user_source_ts
    ON ecg (user_id, source, timestamp DESC);
