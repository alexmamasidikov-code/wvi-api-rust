-- 011_alarms_reminders.sql — Project E

CREATE TABLE user_alarms (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL,
  alarm_id_client UUID NOT NULL,
  time_hhmm TEXT NOT NULL,
  weekday_mask SMALLINT NOT NULL,  -- bit 0=Mon ... bit 6=Sun
  enabled BOOLEAN NOT NULL DEFAULT true,
  label TEXT NOT NULL DEFAULT '',
  smart_wake BOOLEAN NOT NULL DEFAULT false,
  last_modified TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (user_id, alarm_id_client)
);
CREATE INDEX idx_alarms_user ON user_alarms (user_id);

CREATE TABLE user_reminder_settings (
  user_id UUID NOT NULL,
  reminder_type TEXT NOT NULL,  -- 'water'|'stand'|'breathe'|'bedtime'|'move'|'wvi_drop'
  enabled BOOLEAN NOT NULL DEFAULT true,
  start_hour SMALLINT NOT NULL DEFAULT 9,
  end_hour SMALLINT NOT NULL DEFAULT 22,
  min_interval_min INT NOT NULL DEFAULT 120,
  intensity TEXT NOT NULL DEFAULT 'medium',  -- 'light'|'medium'
  last_fired_at TIMESTAMPTZ,
  PRIMARY KEY (user_id, reminder_type),
  CONSTRAINT valid_reminder_type CHECK (reminder_type IN
    ('water','stand','breathe','bedtime','move','wvi_drop')),
  CONSTRAINT valid_intensity CHECK (intensity IN ('light','medium'))
);
CREATE INDEX idx_reminders_user ON user_reminder_settings (user_id);

-- Master switch (global on/off)
CREATE TABLE user_reminder_master (
  user_id UUID PRIMARY KEY,
  enabled BOOLEAN NOT NULL DEFAULT true,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
