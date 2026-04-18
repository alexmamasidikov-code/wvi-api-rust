mod auth;
mod cache;
mod config;
mod error;
mod metrics;
mod validation;
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
mod intraday;
mod alarms;
mod reminders;
mod sensitivity;
mod stress;
mod narrator_schedule;

use cache::AppCache;
use metrics::{spawn_pool_sampler, track_request, Metrics};

use std::sync::Arc;
use axum::{
    routing::{get, post},
    Extension, Router,
};
use sqlx::postgres::PgPoolOptions;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderValue;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{fmt, fmt::time::UtcTime, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use std::time::Duration;
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use axum::middleware::{self as axum_middleware, Next};
use axum::response::Response;
use axum::http::{Request, StatusCode};

use auth::privy::PrivyClient;

fn main() {
    dotenvy::dotenv().ok();

    // Sentry — panic + HTTP + tracing capture. No-op if SENTRY_DSN is unset.
    let _sentry_guard = std::env::var("SENTRY_DSN").ok().map(|dsn| {
        sentry::init((dsn, sentry::ClientOptions {
            release: sentry::release_name!(),
            environment: Some(
                std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()).into()
            ),
            traces_sample_rate: 0.1,
            ..Default::default()
        }))
    });

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime")
        .block_on(async_main());
}

async fn async_main() {
    // OpenTelemetry — OTLP/HTTP span export with 5% head-based sampling.
    // Endpoint: OTEL_EXPORTER_OTLP_ENDPOINT (default http://localhost:4318/v1/traces).
    // Sample ratio override: OTEL_TRACES_SAMPLER_ARG (default 0.05 = 5%).
    // Set OTEL_SDK_DISABLED=true to skip exporter (local dev / tests).
    let otel_layer = init_otel_tracer();

    // Structured logging for Loki/ELK.
    // LOG_FORMAT=json (prod default) → JSON with RFC3339 UTC timer.
    // LOG_FORMAT=pretty (dev default) → human-readable console output.
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".into());
    let default_fmt = if app_env == "production" { "json" } else { "pretty" };
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| default_fmt.into());
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("wvi=info".parse().unwrap());
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(otel_layer)
        .with(sentry_tracing::layer());
    if log_format == "pretty" {
        registry.with(fmt::layer().pretty()).init();
    } else {
        registry.with(fmt::layer().json().with_timer(UtcTime::rfc_3339())).init();
    }

    let cfg = config::Config::from_env();

    // Database pool — sized for 1M user scale
    let max_connections: u32 = std::env::var("DATABASE_MAX_CONNECTIONS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let min_connections: u32 = std::env::var("DATABASE_MIN_CONNECTIONS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let acquire_timeout_secs: u64 = 5;
    let idle_timeout_secs: u64 = 600;

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_secs))
        .idle_timeout(Some(std::time::Duration::from_secs(idle_timeout_secs)))
        .connect(&cfg.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!(
        max_connections,
        min_connections,
        acquire_timeout_secs,
        idle_timeout_secs,
        "PG pool configured for 1M user scale"
    );

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

    // Intraday 5-min downsampler + hourly rollup worker.
    intraday::worker::spawn(pool.clone());

    // Proactive reminders evaluator (Project E) — tick every 5 min, dispatches
    // APNs pushes for the six reminder types when biometric gates + windows
    // + master switch align. No-op if no users have master enabled.
    reminders::evaluator::spawn(pool.clone(), apns.clone());

    // Sensitivity — daily morning/evening AI narrators (Project B). Hourly
    // tick, per-user TZ deferred to Project C.
    sensitivity::narrator::spawn_daily_crons(pool.clone());

    // Project C — emotion + stress inference workers + emotion daily crons.
    emotions::v2::inference::spawn_worker(pool.clone());
    emotions::v2::narrator::spawn_daily_crons(pool.clone());
    stress::v2::inference::spawn_worker(pool.clone());
    stress::v2::micro_pulse::spawn_worker(pool.clone());

    // Metrics collector + periodic DB pool sampler (updates gauges every 5 s).
    let app_metrics = Metrics::new();
    spawn_pool_sampler(pool.clone(), app_metrics.clone());

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
        .route("/api/v1/biometrics/ecg/{id}", get(biometrics::handlers::get_ecg_by_id))
        .route("/api/v1/biometrics/activity", get(biometrics::handlers::get_activity).post(biometrics::handlers::post_activity))
        .route("/api/v1/biometrics/blood-pressure", get(biometrics::handlers::get_blood_pressure).post(biometrics::handlers::post_blood_pressure))
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

        // ═══ WVI v3 (Project C — 18-component personalized) ═══
        .route("/api/v1/wvi/v3/current", get(wvi::v3::handlers::get_current))
        .route("/api/v1/wvi/v3/forecast", get(wvi::v3::handlers::get_forecast))
        .route("/api/v1/wvi/profile", axum::routing::put(wvi::v3::handlers::put_profile))
        .route("/api/v1/wvi/profile-suggest", get(wvi::v3::handlers::get_profile_suggest))

        // ═══ EMOTIONS (8) ═══
        .route("/api/v1/emotions/current", get(emotions::handlers::get_current))
        .route("/api/v1/emotions/history", get(emotions::handlers::get_history))
        .route("/api/v1/emotions/wellbeing", get(emotions::handlers::get_wellbeing))
        .route("/api/v1/emotions/distribution", get(emotions::handlers::get_distribution))
        .route("/api/v1/emotions/heatmap", get(emotions::handlers::get_heatmap))
        .route("/api/v1/emotions/transitions", get(emotions::handlers::get_transitions))
        .route("/api/v1/emotions/triggers", get(emotions::handlers::get_triggers))
        .route("/api/v1/emotions/streaks", get(emotions::handlers::get_streaks))

        // ═══ EMOTIONS v2 (Project C — 1-min 18-label triplet + metrics + narrator) ═══
        .route("/api/v1/emotions/v2/intraday", get(emotions::v2::handlers::get_intraday))
        .route("/api/v1/emotions/v2/metrics", get(emotions::v2::handlers::get_metrics))
        .route("/api/v1/emotions/v2/narrative", get(emotions::v2::handlers::get_narrative))
        .route("/api/v1/emotions/v2/triggers", get(emotions::v2::handlers::get_triggers))

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

        // ═══ TRAINING (5) ═══
        .route("/api/v1/training/recommendation", get(training::handlers::recommendation))
        .route("/api/v1/training/weekly-plan", get(training::handlers::weekly_plan))
        .route("/api/v1/training/overtraining-risk", get(training::handlers::overtraining_risk))
        .route("/api/v1/training/optimal-time", get(training::handlers::optimal_time))
        .route("/api/v1/training/history", get(training::handlers::history))

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

        // ═══ INTRADAY (time-series + backfill) ═══
        .route("/api/v1/intraday", get(intraday::handlers::get_intraday))
        .route("/api/v1/intraday/backfill", post(intraday::handlers::post_backfill))

        // ═══ ALARMS (Project E — app-authoritative + Rust backup) ═══
        .route("/api/v1/alarms/list", get(alarms::handlers::list_alarms))
        .route("/api/v1/alarms/sync", post(alarms::handlers::sync_alarms))
        .route("/api/v1/alarms/{id}", axum::routing::delete(alarms::handlers::delete_alarm))

        // ═══ REMINDERS (Project E — 6 proactive reminder types) ═══
        .route(
            "/api/v1/reminders/settings",
            get(reminders::handlers::get_settings).put(reminders::handlers::put_settings),
        )

        // ═══ STRESS v2 (Project C — 1-min score + 5-level + micro-pulse) ═══
        .route("/api/v1/stress/v2/intraday", get(stress::v2::handlers::get_intraday))

        // ═══ SENSITIVITY (Project B — signals + baselines + contextual AI) ═══
        .route("/api/v1/signals", get(sensitivity::handlers::get_signals))
        .route(
            "/api/v1/signals/{id}/ack",
            axum::routing::put(sensitivity::handlers::ack_signal),
        )
        .route(
            "/api/v1/insights/contextual",
            get(sensitivity::handlers::get_contextual),
        )
        .route("/api/v1/baselines", get(sensitivity::handlers::get_baseline))

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
        .route("/metrics", get(metrics::metrics_handler))

        .layer(Extension(event_bus))
        .layer(Extension(app_cache))
        .layer(Extension(app_metrics))
        .layer(Extension(privy))
        .layer(axum_middleware::from_fn(sentry_middleware))
        .layer(CompressionLayer::new().br(true).gzip(true).deflate(true))
        .layer(TraceLayer::new_for_http())
        .layer(axum_middleware::from_fn(track_request))
        .layer(axum_middleware::from_fn(trace_request_ctx))
        .layer(axum_middleware::from_fn(security_headers))
        .layer(axum_middleware::from_fn(auth::middleware::inject_refresh_hint))
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

    // Flush pending OTel batch spans before process exit.
    opentelemetry::global::shutdown_tracer_provider();
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    tracing::info!("Shutdown signal received");
}

// ─── OpenTelemetry OTLP tracer ───────────────────────────────────────────────
// Exports spans over OTLP/HTTP to the configured collector. Sampler is
// ParentBased(TraceIdRatioBased(ratio)) — root traces are sampled at 5% by
// default; child spans inherit the parent's decision so cross-service traces
// stay consistent. Returns None (layer becomes a no-op) when OTEL_SDK_DISABLED=true
// or the exporter fails to install.
fn init_otel_tracer<S>() -> Option<Box<dyn tracing_subscriber::Layer<S> + Send + Sync + 'static>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span> + Send + Sync,
{
    use opentelemetry::{trace::TracerProvider as _, KeyValue};
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::{trace as sdktrace, Resource};

    if std::env::var("OTEL_SDK_DISABLED").ok().as_deref() == Some("true") {
        return None;
    }

    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4318/v1/traces".into());
    let ratio: f64 = std::env::var("OTEL_TRACES_SAMPLER_ARG")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0.05);
    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "wvi-api-rust".into());

    let exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint.clone())
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary);

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            sdktrace::Config::default()
                .with_sampler(sdktrace::Sampler::ParentBased(Box::new(
                    sdktrace::Sampler::TraceIdRatioBased(ratio),
                )))
                .with_resource(Resource::new(vec![
                    KeyValue::new("service.name", service_name.clone()),
                    KeyValue::new(
                        "deployment.environment",
                        std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
                    ),
                ])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .ok()?;

    let tracer = provider.tracer(service_name.clone());
    opentelemetry::global::set_tracer_provider(provider);

    eprintln!(
        "OpenTelemetry OTLP tracer initialized: endpoint={endpoint} service={service_name} sample_ratio={ratio}"
    );
    Some(Box::new(tracing_opentelemetry::layer().with_tracer(tracer)))
}

// ─── Structured request-context tracing ──────────────────────────────────────
// Attaches `request_id`, `user_id`, `endpoint`, `latency_ms`, `status` on every
// HTTP request as span fields — consumed by the JSON fmt layer for Loki/ELK.
async fn trace_request_ctx(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::extract::MatchedPath;
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Non-reversible user hint — 8-char prefix of Bearer token, never the token itself.
    let user_id = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|t| format!("u_{}", t.chars().take(8).collect::<String>()))
        .unwrap_or_else(|| "anon".to_string());
    let endpoint = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let method = req.method().as_str().to_string();
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        user_id = %user_id,
        endpoint = %endpoint,
        method = %method,
        latency_ms = tracing::field::Empty,
        status = tracing::field::Empty,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    span.record("latency_ms", latency_ms);
    span.record("status", resp.status().as_u16());
    tracing::info!(latency_ms, status = resp.status().as_u16(), "request completed");
    resp
}

// ─── Sentry middleware ───────────────────────────────────────────────────────
// Binds a per-request Sentry Hub (so breadcrumbs don't leak across requests),
// attaches HTTP request context + starts a transaction for performance tracing.
// No-op on the wire when SENTRY_DSN is unset — init() returns a disabled client.

async fn sentry_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use sentry::protocol::{Event, Request as SentryRequest};
    let hub = Arc::new(sentry::Hub::new_from_top(sentry::Hub::current()));
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let tx_name = format!("{method} {}", req.uri().path());
    let tx_ctx = sentry::TransactionContext::new(&tx_name, "http.server");
    let tx = hub.start_transaction(tx_ctx);
    hub.configure_scope(|scope| {
        scope.set_tag("http.method", &method);
        scope.add_event_processor(move |mut event: Event<'static>| {
            event.request.get_or_insert_with(SentryRequest::default).method =
                Some(method.clone());
            event.request.as_mut().unwrap().url = uri.parse().ok();
            Some(event)
        });
    });
    let resp = sentry::Hub::run(hub, || async { next.run(req).await }).await;
    tx.set_status(if resp.status().is_server_error() {
        sentry::protocol::SpanStatus::InternalError
    } else {
        sentry::protocol::SpanStatus::Ok
    });
    tx.finish();
    resp
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

// ─── Per-user rate limiter ───────────────────────────────────────────────────
// Reads: RATE_LIMIT_PER_SEC_READ (default 100) req/s per Bearer token.
// Writes: RATE_LIMIT_PER_SEC_WRITE (default 20) req/s per Bearer token.
// Anonymous (no Bearer): 10 req/s per IP (X-Forwarded-For / X-Real-IP).
// /api/v1/health/* + /metrics bypass all limits. Over quota → 429 + Retry-After: 1.

#[derive(Clone)]
struct RateLimiterState {
    buckets: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    global_count: Arc<AtomicU64>,
    read_limit: u64,
    write_limit: u64,
    anon_limit: u64,
}

fn rate_limiter_state() -> RateLimiterState {
    let read_limit = std::env::var("RATE_LIMIT_PER_SEC_READ")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let write_limit = std::env::var("RATE_LIMIT_PER_SEC_WRITE")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let anon_limit: u64 = 10;
    tracing::info!(read_limit, write_limit, anon_limit, "Rate limiter configured");
    RateLimiterState {
        buckets: Arc::new(Mutex::new(HashMap::new())),
        global_count: Arc::new(AtomicU64::new(0)),
        read_limit, write_limit, anon_limit,
    }
}

async fn rate_limit_middleware(
    Extension(state): Extension<RateLimiterState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response> {
    let path = req.uri().path();
    if path.starts_with("/api/v1/health/") || path == "/metrics" {
        return Ok(next.run(req).await);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let bearer = req.headers().get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(|t| t.to_string()));
    let is_write = matches!(req.method().as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
    let (key, limit) = match bearer {
        Some(tok) => {
            let prefix = if is_write { "w:" } else { "r:" };
            let l = if is_write { state.write_limit } else { state.read_limit };
            (format!("{prefix}{tok}"), l)
        }
        None => {
            let ip = req.headers().get("x-forwarded-for")
                .or_else(|| req.headers().get("x-real-ip"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("anonymous").to_string();
            (format!("a:{ip}"), state.anon_limit)
        }
    };
    {
        let mut buckets = state.buckets.lock().unwrap();
        let entry = buckets.entry(key).or_insert((now, 0));
        if now != entry.0 {
            *entry = (now, 1);
        } else {
            entry.1 += 1;
            if entry.1 > limit {
                let mut resp = Response::new(axum::body::Body::from("Too Many Requests"));
                *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
                resp.headers_mut().insert("Retry-After", "1".parse().unwrap());
                return Err(resp);
            }
        }
        state.global_count.fetch_add(1, Ordering::Relaxed);
        if state.global_count.load(Ordering::Relaxed) % 500 == 0 {
            buckets.retain(|_, (ts, _)| now.saturating_sub(*ts) < 5);
        }
    }
    Ok(next.run(req).await)
}
