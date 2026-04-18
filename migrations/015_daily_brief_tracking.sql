-- 015_daily_brief_tracking.sql — Project G, Task 2
-- Per-user timezone-aware morning/evening brief firing log. The `fired_at`
-- timestamp is the UTC instant the brief went out; the scheduler uses
-- `MAX(fired_at)` per (user, kind) to avoid double-firing inside the same
-- local day.

CREATE TABLE IF NOT EXISTS daily_brief_log (
  user_id UUID NOT NULL,
  kind TEXT NOT NULL,  -- 'morning' | 'evening'
  fired_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (user_id, kind, fired_at),
  CONSTRAINT valid_kind CHECK (kind IN ('morning','evening'))
);

CREATE INDEX IF NOT EXISTS idx_brief_user_kind
  ON daily_brief_log (user_id, kind, fired_at DESC);
