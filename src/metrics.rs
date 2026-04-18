use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::{MatchedPath, Request},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use once_cell::sync::OnceCell;
use prometheus::{
    histogram_opts, opts, CounterVec, Encoder, Gauge, HistogramVec, IntCounterVec, IntGauge,
    Registry, TextEncoder,
};
use sqlx::PgPool;

/// Global handle so deep-module callers (e.g. AI cache) can bump counters
/// without threading the extension through every function signature.
static GLOBAL: OnceCell<Metrics> = OnceCell::new();

pub fn global() -> Option<&'static Metrics> {
    GLOBAL.get()
}

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounterVec,
    pub request_duration: HistogramVec,
    pub db_pool_active: IntGauge,
    pub db_pool_idle: IntGauge,
    pub ai_cache_hits: prometheus::IntCounter,
    pub ai_cache_misses: prometheus::IntCounter,
    pub errors_total: IntCounterVec,
    pub start_time: Instant,
    // Legacy domain counters — wired from handlers.
    pub wvi_calculations: CounterVec,
    pub emotion_detections: prometheus::IntCounter,
    pub biometric_syncs: prometheus::IntCounter,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        let requests_total = IntCounterVec::new(
            opts!("wellex_requests_total", "Total HTTP requests"),
            &["endpoint", "method", "status"],
        )
        .unwrap();
        let request_duration = HistogramVec::new(
            histogram_opts!(
                "wellex_request_duration_seconds",
                "HTTP request latency",
                vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
            ),
            &["endpoint"],
        )
        .unwrap();
        let db_pool_active =
            IntGauge::with_opts(opts!("wellex_db_pool_active", "Active DB connections"))
                .unwrap();
        let db_pool_idle =
            IntGauge::with_opts(opts!("wellex_db_pool_idle", "Idle DB connections")).unwrap();
        let ai_cache_hits = prometheus::IntCounter::with_opts(opts!(
            "wellex_ai_cache_hits_total",
            "AI response cache hits"
        ))
        .unwrap();
        let ai_cache_misses = prometheus::IntCounter::with_opts(opts!(
            "wellex_ai_cache_misses_total",
            "AI response cache misses"
        ))
        .unwrap();
        let errors_total = IntCounterVec::new(
            opts!("wellex_errors_total", "Errors by kind"),
            &["kind"],
        )
        .unwrap();
        let wvi_calculations = CounterVec::new(
            opts!("wellex_wvi_calculations_total", "WVI calculations"),
            &["kind"],
        )
        .unwrap();
        let emotion_detections = prometheus::IntCounter::with_opts(opts!(
            "wellex_emotion_detections_total",
            "Emotion detections"
        ))
        .unwrap();
        let biometric_syncs = prometheus::IntCounter::with_opts(opts!(
            "wellex_biometric_syncs_total",
            "Biometric sync operations"
        ))
        .unwrap();

        registry.register(Box::new(requests_total.clone())).unwrap();
        registry.register(Box::new(request_duration.clone())).unwrap();
        registry.register(Box::new(db_pool_active.clone())).unwrap();
        registry.register(Box::new(db_pool_idle.clone())).unwrap();
        registry.register(Box::new(ai_cache_hits.clone())).unwrap();
        registry.register(Box::new(ai_cache_misses.clone())).unwrap();
        registry.register(Box::new(errors_total.clone())).unwrap();
        registry.register(Box::new(wvi_calculations.clone())).unwrap();
        registry.register(Box::new(emotion_detections.clone())).unwrap();
        registry.register(Box::new(biometric_syncs.clone())).unwrap();

        // Process uptime as a collector-once gauge (set on /metrics scrape).
        let uptime =
            Gauge::with_opts(opts!("wellex_uptime_seconds", "Process uptime")).unwrap();
        registry.register(Box::new(uptime)).unwrap();

        let m = Self {
            registry: Arc::new(registry),
            requests_total,
            request_duration,
            db_pool_active,
            db_pool_idle,
            ai_cache_hits,
            ai_cache_misses,
            errors_total,
            start_time: Instant::now(),
            wvi_calculations,
            emotion_detections,
            biometric_syncs,
        };
        let _ = GLOBAL.set(m.clone());
        m
    }

    pub fn render(&self) -> (HeaderValue, Vec<u8>) {
        let encoder = TextEncoder::new();
        let mut buf = Vec::with_capacity(4096);
        encoder.encode(&self.registry.gather(), &mut buf).ok();
        (
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            buf,
        )
    }
}

/// Axum handler: `GET /metrics` — Prometheus text exposition.
pub async fn metrics_handler(metrics: axum::Extension<Metrics>) -> Response {
    let (ct, body) = metrics.0.render();
    let mut resp = body.into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, ct);
    resp
}

/// Middleware: record latency + status per request, tagged with the matched
/// route template (so high-cardinality ids don't explode the label space).
pub async fn track_request(req: Request<Body>, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let endpoint = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let metrics = req.extensions().get::<Metrics>().cloned();

    let start = Instant::now();
    let resp = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();
    let status = resp.status();

    if let Some(m) = metrics {
        m.requests_total
            .with_label_values(&[&endpoint, &method, status.as_str()])
            .inc();
        m.request_duration
            .with_label_values(&[&endpoint])
            .observe(elapsed);
        if status.is_server_error() {
            m.errors_total.with_label_values(&["5xx"]).inc();
        } else if status == StatusCode::TOO_MANY_REQUESTS {
            m.errors_total.with_label_values(&["rate_limit"]).inc();
        } else if status.is_client_error() {
            m.errors_total.with_label_values(&["4xx"]).inc();
        }
    }
    resp
}

/// Periodic pool-instrumentation task — updates active/idle gauges every 5 s.
pub fn spawn_pool_sampler(pool: PgPool, metrics: Metrics) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let size = pool.size() as i64;
            let idle = pool.num_idle() as i64;
            metrics.db_pool_idle.set(idle);
            metrics.db_pool_active.set((size - idle).max(0));
        }
    });
}
