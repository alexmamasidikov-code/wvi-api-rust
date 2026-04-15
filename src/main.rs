mod auth;
mod config;
mod error;
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

use std::sync::Arc;
use axum::{
    routing::{get, post},
    Extension, Router,
};
use sqlx::postgres::PgPoolOptions;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::privy::PrivyClient;

#[tokio::main]
async fn main() {
    // Init tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "wvi_api=debug,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    dotenvy::dotenv().ok();
    let cfg = config::Config::from_env();

    // Database pool
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&cfg.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("Connected to database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

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
        .allow_origin(Any)
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

        // ═══ HEALTH (3 — PUBLIC) ═══
        .route("/api/v1/health/server-status", get(health::handlers::server_status))
        .route("/api/v1/health/api-version", get(health::handlers::api_version))
        .route("/api/v1/docs.json", get(health::handlers::docs_json))

        .layer(Extension(privy))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(pool);

    let addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("WVI API starting on {addr}");
    tracing::info!("115 endpoints registered across 17 modules");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
