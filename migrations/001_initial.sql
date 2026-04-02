-- WVI Database Schema

-- Users
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    password_hash TEXT NOT NULL,
    age INTEGER,
    gender VARCHAR(10),
    height_cm REAL,
    weight_kg REAL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Personal norms (baselines)
CREATE TABLE IF NOT EXISTS personal_norms (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    resting_hr REAL DEFAULT 65,
    base_temp REAL DEFAULT 36.6,
    avg_hrv REAL DEFAULT 50,
    avg_spo2 REAL DEFAULT 98,
    avg_stress REAL DEFAULT 30,
    avg_systolic REAL DEFAULT 120,
    avg_diastolic REAL DEFAULT 80,
    calibrated_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Heart rate records
CREATE TABLE IF NOT EXISTS heart_rate (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    bpm REAL NOT NULL,
    confidence REAL,
    zone INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_hr_user_ts ON heart_rate(user_id, timestamp DESC);

-- HRV records
CREATE TABLE IF NOT EXISTS hrv (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    sdnn REAL,
    rmssd REAL,
    pnn50 REAL,
    ln_rmssd REAL,
    stress REAL,
    heart_rate REAL,
    systolic_bp REAL,
    diastolic_bp REAL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_hrv_user_ts ON hrv(user_id, timestamp DESC);

-- SpO2 records
CREATE TABLE IF NOT EXISTS spo2 (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    value REAL NOT NULL,
    confidence REAL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_spo2_user_ts ON spo2(user_id, timestamp DESC);

-- Temperature records
CREATE TABLE IF NOT EXISTS temperature (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    value REAL NOT NULL,
    location VARCHAR(20) DEFAULT 'wrist',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_temp_user_ts ON temperature(user_id, timestamp DESC);

-- Sleep records
CREATE TABLE IF NOT EXISTS sleep_records (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    date DATE NOT NULL,
    bedtime TIMESTAMPTZ,
    wake_time TIMESTAMPTZ,
    total_hours REAL,
    sleep_score REAL,
    efficiency REAL,
    deep_percent REAL,
    light_percent REAL,
    rem_percent REAL,
    awake_percent REAL,
    avg_hr REAL,
    avg_hrv REAL,
    avg_spo2 REAL,
    respiratory_rate REAL,
    disturbances INTEGER DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_sleep_user_date ON sleep_records(user_id, date DESC);

-- PPI records
CREATE TABLE IF NOT EXISTS ppi (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    intervals JSONB,
    rmssd REAL,
    coherence REAL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_ppi_user_ts ON ppi(user_id, timestamp DESC);

-- ECG sessions
CREATE TABLE IF NOT EXISTS ecg (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER,
    sample_rate INTEGER,
    samples JSONB,
    analysis JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Activity records
CREATE TABLE IF NOT EXISTS activity (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    steps REAL,
    calories REAL,
    distance REAL,
    active_minutes REAL,
    mets REAL,
    activity_type VARCHAR(50),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_activity_user_ts ON activity(user_id, timestamp DESC);

-- WVI scores
CREATE TABLE IF NOT EXISTS wvi_scores (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    wvi_score REAL NOT NULL,
    level VARCHAR(20) NOT NULL,
    metrics JSONB NOT NULL,
    weights JSONB NOT NULL,
    emotion_feedback REAL DEFAULT 1.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_wvi_user_ts ON wvi_scores(user_id, timestamp DESC);

-- Emotion records
CREATE TABLE IF NOT EXISTS emotions (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    primary_emotion VARCHAR(30) NOT NULL,
    primary_confidence REAL,
    secondary_emotion VARCHAR(30),
    secondary_confidence REAL,
    all_scores JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_emotions_user_ts ON emotions(user_id, timestamp DESC);

-- Alerts
CREATE TABLE IF NOT EXISTS alerts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    level VARCHAR(20) NOT NULL DEFAULT 'info',
    metric VARCHAR(50),
    message TEXT NOT NULL,
    value REAL,
    threshold REAL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    acknowledged BOOLEAN DEFAULT FALSE,
    acknowledged_at TIMESTAMPTZ
);
CREATE INDEX idx_alerts_user ON alerts(user_id, created_at DESC);

-- Alert settings
CREATE TABLE IF NOT EXISTS alert_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    enabled BOOLEAN DEFAULT TRUE,
    thresholds JSONB DEFAULT '{}',
    channels JSONB DEFAULT '{"push": true, "email": false, "sms": false}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Reports
CREATE TABLE IF NOT EXISTS reports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    template VARCHAR(50) NOT NULL,
    title TEXT,
    status VARCHAR(20) DEFAULT 'generating',
    content JSONB,
    file_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- App settings
CREATE TABLE IF NOT EXISTS app_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    units VARCHAR(10) DEFAULT 'metric',
    language VARCHAR(10) DEFAULT 'en',
    timezone VARCHAR(50) DEFAULT 'UTC',
    theme VARCHAR(10) DEFAULT 'auto',
    data_retention INTEGER DEFAULT 365,
    privacy JSONB DEFAULT '{"shareAnonymousData": false, "showInLeaderboard": false}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Notification settings
CREATE TABLE IF NOT EXISTS notification_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    push BOOLEAN DEFAULT TRUE,
    email BOOLEAN DEFAULT FALSE,
    sms BOOLEAN DEFAULT FALSE,
    quiet_hours JSONB DEFAULT '{"enabled": false, "start": "22:00", "end": "07:00"}',
    alert_levels JSONB DEFAULT '{"critical": true, "warning": true, "info": false, "notice": false}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Devices
CREATE TABLE IF NOT EXISTS devices (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    device_name VARCHAR(100),
    firmware_version VARCHAR(50),
    battery_level INTEGER,
    last_sync_at TIMESTAMPTZ,
    capabilities JSONB DEFAULT '{}',
    auto_monitoring JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
