# Architecture Refactor Phase 1 — API Stability & Scalability

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix critical scalability bottlenecks in the Rust API — eliminate N+1 queries, add batch inserts, proper DB migrations, auth caching, and connection pool scaling.

**Architecture:** Extract user_id resolution into middleware (run once per request, not per handler). Convert single-row INSERTs to batch operations. Replace runtime CREATE TABLE with proper SQL migrations. Add in-memory auth cache with TTL.

**Tech Stack:** Rust, Axum 0.8, SQLx 0.8, PostgreSQL, Tokio

---

## File Structure

### Modified files:
- `src/main.rs` — add migration runner, increase pool size, add auth cache layer
- `src/auth/middleware.rs` — cache user UUID after first lookup
- `src/biometrics/handlers.rs` — batch inserts, remove N+1 user lookups
- `src/wvi/handlers.rs` — use cached user_id
- `src/emotions/handlers.rs` — use cached user_id
- `src/ai/handlers.rs` — add response caching

### New files:
- `migrations/001_init.sql` — all CREATE TABLE statements moved here
- `src/cache.rs` — in-memory cache for auth + AI responses

---

### Task 1: Create proper SQL migrations

**Files:**
- Create: `migrations/001_init.sql`
- Modify: `src/main.rs`

- [ ] **Step 1: Extract all CREATE TABLE statements from main.rs**

Read `src/main.rs`, find all `CREATE TABLE IF NOT EXISTS` statements. Move them to `migrations/001_init.sql`:

```sql
-- migrations/001_init.sql
-- WVIHealth Database Schema

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    privy_did TEXT UNIQUE NOT NULL,
    email TEXT,
    name TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Add indexes that are missing
CREATE INDEX IF NOT EXISTS idx_users_privy ON users(privy_did);

CREATE TABLE IF NOT EXISTS heart_rate (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    timestamp TIMESTAMPTZ NOT NULL,
    bpm REAL NOT NULL,
    confidence REAL,
    zone INTEGER,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_hr_user_ts ON heart_rate(user_id, timestamp DESC);

-- ... all other tables with proper indexes
```

- [ ] **Step 2: Add migration runner to main.rs**

Replace inline CREATE TABLE calls with:
```rust
// Run migrations
sqlx::migrate!("./migrations").run(&pool).await?;
tracing::info!("Database migrations complete");
```

- [ ] **Step 3: Add missing indexes**

```sql
-- Add to migrations/001_init.sql
CREATE INDEX IF NOT EXISTS idx_spo2_user_ts ON spo2(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_hrv_user_ts ON hrv(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_temp_user_ts ON temperature(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_activity_user_ts ON activity(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_emotions_user_ts ON emotions(user_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_wvi_user_calc ON wvi_scores(user_id, calculated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sleep_user_date ON sleep_records(user_id, date DESC);
CREATE INDEX IF NOT EXISTS idx_social_created ON social_posts(created_at DESC);
```

- [ ] **Step 4: Build and test**

```bash
cargo build 2>&1 | grep -E "^error" | head -5
cargo run &
sleep 3
curl -s http://localhost:8091/api/v1/health/server-status
```

- [ ] **Step 5: Commit**

```bash
git add migrations/ src/main.rs
git commit -m "refactor: extract DB schema to migrations, add missing indexes"
```

---

### Task 2: Cache user_id in auth middleware (eliminate N+1)

**Files:**
- Create: `src/cache.rs`
- Modify: `src/auth/middleware.rs`
- Modify: `src/biometrics/handlers.rs`

- [ ] **Step 1: Create in-memory cache module**

Create `src/cache.rs`:
```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Instant, Duration};

#[derive(Clone)]
pub struct AppCache {
    user_ids: Arc<RwLock<HashMap<String, (uuid::Uuid, Instant)>>>,
    ai_responses: Arc<RwLock<HashMap<String, (String, Instant)>>>,
}

impl AppCache {
    pub fn new() -> Self {
        Self {
            user_ids: Arc::new(RwLock::new(HashMap::new())),
            ai_responses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get cached user UUID, or None if expired/missing
    pub async fn get_user_id(&self, privy_did: &str) -> Option<uuid::Uuid> {
        let cache = self.user_ids.read().await;
        cache.get(privy_did).and_then(|(id, ts)| {
            if ts.elapsed() < Duration::from_secs(300) { Some(*id) } else { None }
        })
    }

    /// Cache user UUID for 5 minutes
    pub async fn set_user_id(&self, privy_did: &str, id: uuid::Uuid) {
        let mut cache = self.user_ids.write().await;
        cache.insert(privy_did.to_string(), (id, Instant::now()));
    }

    /// Get cached AI response, or None if expired
    pub async fn get_ai(&self, key: &str) -> Option<String> {
        let cache = self.ai_responses.read().await;
        cache.get(key).and_then(|(resp, ts)| {
            if ts.elapsed() < Duration::from_secs(600) { Some(resp.clone()) } else { None }
        })
    }

    /// Cache AI response for 10 minutes
    pub async fn set_ai(&self, key: &str, response: String) {
        let mut cache = self.ai_responses.write().await;
        cache.insert(key.to_string(), (response, Instant::now()));
    }
}
```

- [ ] **Step 2: Register cache in main.rs app state**

In `src/main.rs`, add cache to app state:
```rust
mod cache;
use cache::AppCache;

// In main():
let app_cache = AppCache::new();

// Add to all routes as extension:
let app = Router::new()
    // ... routes ...
    .layer(Extension(app_cache))
    .with_state(pool);
```

- [ ] **Step 3: Update get_user_uuid to use cache**

In `src/biometrics/handlers.rs`, update `get_user_uuid`:
```rust
pub async fn get_user_uuid(pool: &PgPool, privy_did: &str, cache: Option<&AppCache>) -> AppResult<uuid::Uuid> {
    // Check cache first
    if let Some(c) = cache {
        if let Some(id) = c.get_user_id(privy_did).await {
            return Ok(id);
        }
    }

    let id = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1")
        .bind(privy_did)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))?;

    // Cache the result
    if let Some(c) = cache {
        c.set_user_id(privy_did, id).await;
    }

    Ok(id)
}
```

- [ ] **Step 4: Build and test**
- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/auth/ src/biometrics/ src/main.rs
git commit -m "perf: add in-memory user_id cache, eliminate N+1 queries"
```

---

### Task 3: Batch inserts for biometric sync

**Files:**
- Modify: `src/biometrics/handlers.rs`

- [ ] **Step 1: Replace single INSERTs with batch**

In the `sync` handler, replace individual INSERT loops with batch:
```rust
// Collect all heart_rate records
let mut hr_values: Vec<(uuid::Uuid, chrono::DateTime<Utc>, f32)> = vec![];

for rec in &body.records {
    match rec.record_type.as_str() {
        "heart_rate" => {
            if let Some(bpm) = rec.data.get("bpm").and_then(|v| v.as_f64()) {
                hr_values.push((uid, rec.timestamp, bpm as f32));
            }
        }
        // ... other types
    }
}

// Batch insert heart_rate
if !hr_values.is_empty() {
    let mut query = String::from("INSERT INTO heart_rate (user_id, timestamp, bpm) VALUES ");
    let mut params: Vec<String> = vec![];
    for (i, (uid, ts, bpm)) in hr_values.iter().enumerate() {
        let base = i * 3 + 1;
        params.push(format!("(${}, ${}, ${})", base, base + 1, base + 2));
    }
    query.push_str(&params.join(", "));

    let mut q = sqlx::query(&query);
    for (uid, ts, bpm) in &hr_values {
        q = q.bind(uid).bind(ts).bind(bpm);
    }
    q.execute(&pool).await?;
    processed += hr_values.len();
}
```

- [ ] **Step 2: Build and test**
- [ ] **Step 3: Commit**

---

### Task 4: Increase DB pool + connection settings

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Update pool configuration**

```rust
let pool = PgPoolOptions::new()
    .max_connections(100)           // was 20
    .min_connections(5)             // keep 5 warm
    .acquire_timeout(Duration::from_secs(10))
    .idle_timeout(Duration::from_secs(300))
    .max_lifetime(Duration::from_secs(1800))
    .connect(&database_url)
    .await?;
```

- [ ] **Step 2: Commit**

---

### Task 5: AI response caching

**Files:**
- Modify: `src/ai/handlers.rs`

- [ ] **Step 1: Add cache to AI handlers**

Use AppCache to cache AI responses by prompt hash:
```rust
pub async fn interpret(
    user: AuthUser,
    State(pool): State<PgPool>,
    Extension(cache): Extension<AppCache>,
    Json(_body): Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let cache_key = format!("ai:interpret:{}", user.privy_did);

    // Check cache
    if let Some(cached) = cache.get_ai(&cache_key).await {
        return Ok(Json(serde_json::json!({ "success": true, "data": { "message": cached, "cached": true } })));
    }

    let prompt = "...";
    match call_claude(&pool, &user.privy_did, prompt).await {
        Ok(text) => {
            cache.set_ai(&cache_key, text.clone()).await;
            Ok(Json(serde_json::json!({ "success": true, "data": { "message": text } })))
        }
        Err(e) => Ok(Json(serde_json::json!({ "success": false, "data": { "message": e } }))),
    }
}
```

- [ ] **Step 2: Build and test**
- [ ] **Step 3: Commit**

---

### Task 6: Production configuration

**Files:**
- Create: `.env.production`
- Modify: `src/main.rs`

- [ ] **Step 1: Create production env template**

```env
# .env.production
DATABASE_URL=postgres://wvi:STRONG_PASSWORD@db.wellex.ai:5432/wvi
JWT_SECRET=GENERATE_RANDOM_64_CHAR
PORT=8091
CLAUDE_API_KEY=sk-or-v1-...
CLAUDE_MODEL=google/gemini-2.0-flash-001
CLAUDE_API_URL=https://openrouter.ai/api/v1/chat/completions
RUST_LOG=info
MAX_DB_CONNECTIONS=100
```

- [ ] **Step 2: Read config from env**

```rust
let max_connections: u32 = std::env::var("MAX_DB_CONNECTIONS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(20);
```

- [ ] **Step 3: Commit**

---

### Task 7: Full build + test

- [ ] **Step 1: Build**
```bash
cargo build --release 2>&1 | grep -E "^error" | head -5
```

- [ ] **Step 2: Run**
```bash
cargo run &
sleep 3
curl -s http://localhost:8091/api/v1/health/server-status
```

- [ ] **Step 3: Test all endpoints still work**

- [ ] **Step 4: Final commit**
```bash
git add -A
git commit -m "refactor: Phase 1 complete — migrations, cache, batch inserts, pool scaling"
```
