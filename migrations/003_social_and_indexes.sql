-- Social tables (moved from main.rs inline CREATE TABLE)
CREATE TABLE IF NOT EXISTS social_posts (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    content TEXT NOT NULL,
    likes INT DEFAULT 0,
    comments INT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS challenges (
    id BIGSERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT,
    target_value REAL,
    start_date DATE,
    end_date DATE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS challenge_participants (
    id BIGSERIAL PRIMARY KEY,
    challenge_id BIGINT REFERENCES challenges(id),
    user_id UUID REFERENCES users(id),
    progress REAL DEFAULT 0,
    joined_at TIMESTAMPTZ DEFAULT NOW()
);

-- Critical performance indexes
CREATE INDEX IF NOT EXISTS idx_hr_user_ts ON heart_rate(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_spo2_user_ts ON spo2(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_hrv_user_ts ON hrv(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_temp_user_ts ON temperature(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_activity_user_ts ON activity(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emotions_user_ts ON emotions(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_wvi_user_ts ON wvi_scores(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_sleep_user_date ON sleep_records(user_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_social_created ON social_posts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_users_privy ON users(privy_did);
