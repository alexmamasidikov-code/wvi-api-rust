mod auth;
mod cache;
mod config;
mod error;
mod metrics;
mod users;
mod biometrics;
mod wvi;
mod emotions;
mod activities;
mod sleep;
mod ai;
mod reports;
mod alerts;
mod device;
mod training;
mod risk;
mod dashboard;
mod export;
mod settings;
mod health;
mod social;
mod family;
mod audit;
mod events;
mod push;

use cache::AppCache;
use metrics::Metrics;

use std::sync::Arc;
use axum::{
    routing::{get, post},
    Extension, Router,
};
use sqlx::postgres::PgPoolOptions;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use std::time::Duration;
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use axum::middleware::{self as axum_middleware, Next};
use axum::response::Response;
use axum::http::{Request, StatusCode};

use auth::privy::PrivyClient;

#[tokio::main]
async fn main() {
    // Init tracing — structured JSON logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();
    let cfg = config::Config::from_env();

    // Database pool
    let max_connections: u32 = std::env::var("MAX_DB_CONNECTIONS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(20);

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .idle_timeout(std::time::Duration::from_secs(300))
        .connect(&cfg.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("Connected to database (max_connections={max_connections})");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // In-memory cache
    let app_cache = AppCache::new();

    // Background AI panel prewarmer — keeps daily_brief / recovery_deep /
    // full_analysis / ecg_interpret / evening_review / weekly_deep cached
    // for every user with recent biometric activity. Users tapping a card
    // get instant responses instead of waiting 20-40 s for the CLI.
    ai::precompute::spawn_prewarmer(pool.clone(), app_cache.clone());

    // APNs client + push scheduler. No-op if APNS_* env vars are missing
    // (client logs a one-time warning and all send() calls return Ok).
    let apns = push::apns::ApnsClient::new();
    push::scheduler::spawn_scheduler(pool.clone(), app_cache.clone(), apns.clone());

    // Metrics collector
    let app_metrics = Metrics::new();

    // Seed challenges
    sqlx::query("INSERT INTO challenges (title, description, target_value, start_date, end_date) VALUES ('10K Steps Daily', 'Walk 10,000 steps every day', 10000, CURRENT_DATE, CURRENT_DATE + 7) ON CONFLICT DO NOTHING").execute(&pool).await.ok();
    sqlx::query("INSERT INTO challenges (title, description, target_value, start_date, end_date) VALUES ('Sleep Score 80+', 'Achieve sleep score above 80 for a week', 80, CURRENT_DATE, CURRENT_DATE + 7) ON CONFLICT DO NOTHING").execute(&pool).await.ok();
    sqlx::query("INSERT INTO challenges (title, description, target_value, start_date, end_date) VALUES ('HRV Improvement', 'Improve your HRV by 10% this week', 10, CURRENT_DATE, CURRENT_DATE + 7) ON CONFLICT DO NOTHING").execute(&pool).await.ok();

    // Kafka event bus
    let event_bus = match std::env::var("KAFKA_BROKERS") {
        Ok(brokers) => {
            match events::EventBus::new(&brokers) {
                Ok(bus) => {
                    tracing::info!("Kafka event bus connected to {brokers}");
                    bus
                }
                Err(e) => {
                    tracing::warn!("Failed to connect to Kafka at {brokers}: {e} — using noop event bus");
                    events::EventBus::noop()
                }
            }
        }
        Err(_) => {
            tracing::warn!("KAFKA_BROKERS not set — using noop event bus");
            events::EventBus::noop()
        }
    };

    // Privy client
    let privy = Arc::new(PrivyClient::new(
        cfg.privy_app_id.clone(),
        cfg.privy_app_secret.clone(),
    ));
    if privy.is_configured() {
        tracing::info!("Privy auth configured");
    } else {
        tracing::warn!("Privy not configured — using dev mode auth");
        sqlx::query(
            r#"INSERT INTO users (id, email, name, password_hash, privy_did, linked_accounts, created_at, updated_at)
               VALUES (gen_random_uuid(), 'dev@wvi.health', 'Alexander', '', 'did:privy:dev-user', '[]'::jsonb, NOW(), NOW())
               ON CONFLICT (privy_did) DO UPDATE SET
                 email = EXCLUDED.email,
                 name = EXCLUDED.name,
                 updated_at = NOW()"#,
        )
        .execute(&pool)
        .await
        .expect("Failed to bootstrap dev user");
    }

    let cors = CorsLayer::new()
        .allow_origin([
            "https://6ssssdj5s38h.share.zrok.io".parse::<HeaderValue>().unwrap(),
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // ═══ AUTH — Privy (4) ═══
        .route("/api/v1/auth/verify", post(auth::handlers::verify))
        .route("/api/v1/auth/me", get(auth::handlers::me))
        .route("/api/v1/auth/link-wallet", post(auth::handlers::link_wallet))
        .route("/api/v1/auth/logout", post(auth::handlers::logout))

        // ═══ USERS (4) ═══
        .route("/api/v1/users/me", get(users::handlers::get_me).put(users::handlers::update_me))
        .route("/api/v1/users/me/norms", get(users::handlers::get_norms))
        .route("/api/v1/users/me/norms/calibrate", post(users::handlers::calibrate))

        // ═══ BIOMETRICS (18+) ═══
        .route("/api/v1/biometrics/sync", post(biometrics::handlers::sync))
        .route("/api/v1/biometrics/heart-rate", get(biometrics::handlers::get_heart_rate).post(biometrics::handlers::post_heart_rate))
        .route("/api/v1/biometrics/hrv", get(biometrics::handlers::get_hrv).post(biometrics::handlers::post_hrv))
        .route("/api/v1/biometrics/spo2", get(biometrics::handlers::get_spo2).post(biometrics::handlers::post_spo2))
        .route("/api/v1/biometrics/temperature", get(biometrics::handlers::get_temperature).post(biometrics::handlers::post_temperature))
        .route("/api/v1/biometrics/sleep", get(biometrics::handlers::get_sleep).post(biometrics::handlers::post_sleep))
        .route("/api/v1/biometrics/ppi", get(biometrics::handlers::get_ppi).post(biometrics::handlers::post_ppi))
        .route("/api/v1/biometrics/ecg", get(biometrics::handlers::get_ecg).post(biometrics::handlers::post_ecg))
        .route("/api/v1/biometrics/activity", get(biometrics::handlers::get_activity).post(biometrics::handlers::post_activity))
        .route("/api/v1/biometrics/blood-pressure", get(biometrics::handlers::get_blood_pressure))
        .route("/api/v1/biometrics/stress", get(biometrics::handlers::get_stress))
        .route("/api/v1/biometrics/breathing-rate", get(biometrics::handlers::get_breathing_rate))
        .route("/api/v1/biometrics/rmssd", get(biometrics::handlers::get_rmssd))
        .route("/api/v1/biometrics/coherence", get(biometrics::handlers::get_coherence))
        .route("/api/v1/biometrics/computed", get(biometrics::handlers::get_computed))
        .route("/api/v1/biometrics/recovery", get(biometrics::handlers::get_recovery))
        .route("/api/v1/biometrics/realtime", get(biometrics::handlers::get_realtime))
        .route("/api/v1/biometrics/summary", get(biometrics::handlers::get_summary))

        // ═══ WVI (10) ═══
        .route("/api/v1/wvi/current", get(wvi::handlers::get_current))
        .route("/api/v1/wvi/history", get(wvi::handlers::get_history))
        .route("/api/v1/wvi/trends", get(wvi::handlers::get_trends))
        .route("/api/v1/wvi/predict", get(wvi::handlers::predict))
        .route("/api/v1/wvi/simulate", post(wvi::handlers::simulate))
        .route("/api/v1/wvi/circadian", get(wvi::handlers::circadian))
        .route("/api/v1/wvi/correlations", get(wvi::handlers::correlations))
        .route("/api/v1/wvi/breakdown", get(wvi::handlers::breakdown))
        .route("/api/v1/wvi/compare", get(wvi::handlers::compare))

        // ═══ EMOTIONS (8) ═══
        .route("/api/v1/emotions/current", get(emotions::handlers::get_current))
        .route("/api/v1/emotions/history", get(emotions::handlers::get_history))
        .route("/api/v1/emotions/wellbeing", get(emotions::handlers::get_wellbeing))
        .route("/api/v1/emotions/distribution", get(emotions::handlers::get_distribution))
        .route("/api/v1/emotions/heatmap", get(emotions::handlers::get_heatmap))
        .route("/api/v1/emotions/transitions", get(emotions::handlers::get_transitions))
        .route("/api/v1/emotions/triggers", get(emotions::handlers::get_triggers))
        .route("/api/v1/emotions/streaks", get(emotions::handlers::get_streaks))

        // ═══ ACTIVITIES (10) ═══
        .route("/api/v1/activities/current", get(activities::handlers::get_current))
        .route("/api/v1/activities/history", get(activities::handlers::get_history))
        .route("/api/v1/activities/load", get(activities::handlers::get_load))
        .route("/api/v1/activities/zones", get(activities::handlers::get_zones))
        .route("/api/v1/activities/categories", get(activities::handlers::get_categories))
        .route("/api/v1/activities/transitions", get(activities::handlers::get_transitions))
        .route("/api/v1/activities/sedentary", get(activities::handlers::get_sedentary))
        .route("/api/v1/activities/exercise-log", get(activities::handlers::get_exercise_log))
        .route("/api/v1/activities/recovery-status", get(activities::handlers::get_recovery_status))
        .route("/api/v1/activities/manual-log", post(activities::handlers::manual_log))

        // ═══ SLEEP (7) ═══
        .route("/api/v1/sleep/last-night", get(sleep::handlers::last_night))
        .route("/api/v1/sleep/score-history", get(sleep::handlers::score_history))
        .route("/api/v1/sleep/architecture", get(sleep::handlers::architecture))
        .route("/api/v1/sleep/consistency", get(sleep::handlers::consistency))
        .route("/api/v1/sleep/debt", get(sleep::handlers::debt))
        .route("/api/v1/sleep/phases", get(sleep::handlers::phases))
        .route("/api/v1/sleep/optimal-window", get(sleep::handlers::optimal_window))

        // ═══ AI (7) ═══
        .route("/api/v1/ai/interpret", post(ai::handlers::interpret))
        .route("/api/v1/ai/recommendations", post(ai::handlers::recommendations))
        .route("/api/v1/ai/chat", post(ai::handlers::chat))
        .route("/api/v1/ai/explain-metric", post(ai::handlers::explain_metric))
        .route("/api/v1/ai/action-plan", post(ai::handlers::action_plan))
        .route("/api/v1/ai/insights", post(ai::handlers::insights))
        .route("/api/v1/ai/genius-layer", post(ai::handlers::genius_layer))
        // AI Coach 2.0 proactive
        .route("/api/v1/ai/daily-brief", post(ai::handlers::daily_brief))
        .route("/api/v1/ai/evening-review", post(ai::handlers::evening_review))
        .route("/api/v1/ai/anomaly-alert", post(ai::handlers::anomaly_alert))
        .route("/api/v1/ai/weekly-deep", post(ai::handlers::weekly_deep))
        // Medical-analyst tier
        .route("/api/v1/ai/full-analysis", post(ai::handlers::full_analysis))
        .route("/api/v1/ai/ecg-interpret", post(ai::handlers::ecg_interpret))
        .route("/api/v1/ai/recovery-deep", post(ai::handlers::recovery_deep))

        // ═══ REPORTS (5) ═══
        .route("/api/v1/reports/generate", post(reports::handlers::generate))
        .route("/api/v1/reports/list", get(reports::handlers::list))
        .route("/api/v1/reports/templates", get(reports::handlers::get_templates))
        .route("/api/v1/reports/{id}", get(reports::handlers::get_by_id))
        .route("/api/v1/reports/{id}/download", get(reports::handlers::download))

        // ═══ ALERTS (6) ═══
        .route("/api/v1/alerts/list", get(alerts::handlers::list))
        .route("/api/v1/alerts/active", get(alerts::handlers::active))
        .route("/api/v1/alerts/settings", get(alerts::handlers::get_settings))
        .route("/api/v1/alerts/history", get(alerts::handlers::get_history))
        .route("/api/v1/alerts/{id}/acknowledge", post(alerts::handlers::acknowledge))
        .route("/api/v1/alerts/stats", get(alerts::handlers::stats))

        // ═══ DEVICE (6) ═══
        .route("/api/v1/device/status", get(device::handlers::status))
        .route("/api/v1/device/auto-monitoring", post(device::handlers::auto_monitoring))
        .route("/api/v1/device/sync", post(device::handlers::sync))
        .route("/api/v1/device/capabilities", get(device::handlers::capabilities))
        .route("/api/v1/device/measure", post(device::handlers::measure))
        .route("/api/v1/device/firmware", get(device::handlers::firmware))

        // ═══ TRAINING (4) ═══
        .route("/api/v1/training/recommendation", get(training::handlers::recommendation))
        .route("/api/v1/training/weekly-plan", get(training::handlers::weekly_plan))
        .route("/api/v1/training/overtraining-risk", get(training::handlers::overtraining_risk))
        .route("/api/v1/training/optimal-time", get(training::handlers::optimal_time))

        // ═══ RISK (5) ═══
        .route("/api/v1/risk/assessment", get(risk::handlers::assessment))
        .route("/api/v1/risk/anomalies", get(risk::handlers::anomalies))
        .route("/api/v1/risk/chronic-flags", get(risk::handlers::chronic_flags))
        .route("/api/v1/risk/correlations", get(risk::handlers::correlations))
        .route("/api/v1/risk/volatility", get(risk::handlers::volatility))

        // ═══ DASHBOARD (3) ═══
        .route("/api/v1/dashboard/widgets", get(dashboard::handlers::widgets))
        .route("/api/v1/dashboard/daily-brief", get(dashboard::handlers::daily_brief))
        .route("/api/v1/dashboard/evening-review", get(dashboard::handlers::evening_review))

        // ═══ EXPORT (3) ═══
        .route("/api/v1/export/csv", get(export::handlers::csv_export))
        .route("/api/v1/export/json", get(export::handlers::json_export))
        .route("/api/v1/export/health-summary", get(export::handlers::health_summary))

        // ═══ SETTINGS (4) ═══
        .route("/api/v1/settings", get(settings::handlers::get_settings).put(settings::handlers::update_settings))
        .route("/api/v1/settings/notifications", get(settings::handlers::get_notifications).put(settings::handlers::update_notifications))

        // ═══ PUSH (APNs) ═══
        .route("/api/v1/notifications/register", post(push::handlers::register_token))

        // ═══ AUDIT (1) ═══
        .route("/api/v1/audit/log", get(audit::get_audit_log))

        // ═══ SOCIAL (4) ═══
        .route("/api/v1/social/feed", get(social::handlers::get_feed))
        .route("/api/v1/social/post", post(social::handlers::create_post))
        .route("/api/v1/social/challenges", get(social::handlers::get_challenges))
        .route("/api/v1/social/leaderboard", get(social::handlers::get_leaderboard))

        // ═══ FAMILY (5 — PostgreSQL) ═══
        .route("/api/v1/family/members", get(family::handlers::members))
        .route("/api/v1/family/average", get(family::handlers::average))
        .route("/api/v1/family/alerts", get(family::handlers::alerts))
        .route("/api/v1/family/invite", post(family::handlers::invite))
        .route("/api/v1/family/accept/:id", post(family::handlers::accept))

        // ═══ HEALTH (5 — PUBLIC) ═══
        .route("/api/v1/health/server-status", get(health::handlers::server_status))
        .route("/api/v1/health/api-version", get(health::handlers::api_version))
        .route("/api/v1/health/ready", get(health::handlers::readiness))
        .route("/api/v1/health/live", get(health::handlers::liveness))
        .route("/api/v1/docs.json", get(health::handlers::docs_json))

        // ═══ METRICS (Prometheus) ═══
        .route("/metrics", get({
            let m = app_metrics.clone();
            move || async move { m.to_prometheus() }
        }))

        .layer(Extension(event_bus))
        .layer(Extension(app_cache))
        .layer(Extension(app_metrics))
        .layer(Extension(privy))
        .layer(TraceLayer::new_for_http())
        .layer(axum_middleware::from_fn(security_headers))
        .layer(cors)
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .layer(axum_middleware::from_fn(rate_limit_middleware))
        .layer(Extension(rate_limiter_state()))
        .with_state(pool);

    let addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("WVI API starting on {addr}");
    tracing::info!("123 endpoints registered across 18 modules");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    tracing::info!("Shutdown signal received");
}

// ─── Security headers middleware ─────────────────────────────────────────────

async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    headers.insert("Referrer-Policy", "strict-origin-when-cross-origin".parse().unwrap());
    response
}

// ─── Per-user rate limiter (60 req/min per user, 20 req/min unauthenticated) ─

#[derive(Clone)]
struct RateLimiterState {
    /// Per-key buckets: key → (window_start_sec, request_count)
    buckets: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    /// Global request counter for metrics
    global_count: Arc<AtomicU64>,
}

fn rate_limiter_state() -> RateLimiterState {
    RateLimiterState {
        buckets: Arc::new(Mutex::new(HashMap::new())),
        global_count: Arc::new(AtomicU64::new(0)),
    }
}

async fn rate_limit_middleware(
    Extension(state): Extension<RateLimiterState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let window_secs = 60; // 1 minute window

    // Increment global request counter for metrics
    if let Some(m) = req.extensions().get::<Metrics>() {
        m.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    // Extract user identity: prefer user_id from auth, fallback to IP
    let user_key = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            req.headers()
                .get("x-forwarded-for")
                .or_else(|| req.headers().get("x-real-ip"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("anonymous")
                .to_string()
        });

    let is_authenticated = req.headers().get("Authorization").is_some();
    let limit: u64 = if is_authenticated { 60 } else { 20 };

    // Check and update per-user bucket
    {
        let mut buckets = state.buckets.lock().unwrap();
        let entry = buckets.entry(user_key).or_insert((now, 0));
        if now - entry.0 >= window_secs {
            // New window
            *entry = (now, 1);
        } else {
            entry.1 += 1;
            if entry.1 > limit {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }

        // Periodic cleanup: remove stale entries (every ~100 requests)
        state.global_count.fetch_add(1, Ordering::Relaxed);
        if state.global_count.load(Ordering::Relaxed) % 100 == 0 {
            buckets.retain(|_, (ts, _)| now - *ts < window_secs * 2);
        }
    }

    Ok(next.run(req).await)
}
