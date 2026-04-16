use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct Metrics {
    pub requests_total: Arc<AtomicU64>,
    pub requests_success: Arc<AtomicU64>,
    pub requests_error: Arc<AtomicU64>,
    pub wvi_calculations: Arc<AtomicU64>,
    pub emotion_detections: Arc<AtomicU64>,
    pub biometric_syncs: Arc<AtomicU64>,
    pub start_time: Instant,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            requests_total: Arc::new(AtomicU64::new(0)),
            requests_success: Arc::new(AtomicU64::new(0)),
            requests_error: Arc::new(AtomicU64::new(0)),
            wvi_calculations: Arc::new(AtomicU64::new(0)),
            emotion_detections: Arc::new(AtomicU64::new(0)),
            biometric_syncs: Arc::new(AtomicU64::new(0)),
            start_time: Instant::now(),
        }
    }

    pub fn to_prometheus(&self) -> String {
        let uptime = self.start_time.elapsed().as_secs();
        format!(
            "# HELP wvi_requests_total Total HTTP requests\n\
             # TYPE wvi_requests_total counter\n\
             wvi_requests_total {}\n\
             # HELP wvi_requests_success Successful requests\n\
             # TYPE wvi_requests_success counter\n\
             wvi_requests_success {}\n\
             # HELP wvi_requests_error Failed requests\n\
             # TYPE wvi_requests_error counter\n\
             wvi_requests_error {}\n\
             # HELP wvi_calculations_total WVI score calculations\n\
             # TYPE wvi_calculations_total counter\n\
             wvi_calculations_total {}\n\
             # HELP wvi_emotion_detections_total Emotion detections\n\
             # TYPE wvi_emotion_detections_total counter\n\
             wvi_emotion_detections_total {}\n\
             # HELP wvi_biometric_syncs_total Biometric sync operations\n\
             # TYPE wvi_biometric_syncs_total counter\n\
             wvi_biometric_syncs_total {}\n\
             # HELP wvi_uptime_seconds Server uptime\n\
             # TYPE wvi_uptime_seconds gauge\n\
             wvi_uptime_seconds {}\n",
            self.requests_total.load(Ordering::Relaxed),
            self.requests_success.load(Ordering::Relaxed),
            self.requests_error.load(Ordering::Relaxed),
            self.wvi_calculations.load(Ordering::Relaxed),
            self.emotion_detections.load(Ordering::Relaxed),
            self.biometric_syncs.load(Ordering::Relaxed),
            uptime,
        )
    }
}
