// Derived metric helpers.
// Uses existing WVI v1 normalizer for now; Project C will replace with v3.
use crate::intraday::types::MetricType;

/// For a derived metric, compute its value given a map of raw-metric means
/// over the 5-minute bucket. Returns None if inputs missing.
pub fn compute_derived(
    metric: MetricType,
    _raw_means: &std::collections::HashMap<&str, f64>,
) -> Option<f64> {
    match metric {
        MetricType::Wvi => {
            // Simplified passthrough — existing wvi computation happens elsewhere.
            // Here we just read last computed WVI sample from biometrics_1min
            // upstream, so this returns None to skip rebucket for derived.
            None
        }
        MetricType::Stress
        | MetricType::EmotionConfidence
        | MetricType::Energy
        | MetricType::Recovery
        | MetricType::Coherence => {
            // Same: derived values are already written via write_1min from
            // computation elsewhere (compute_wvi trigger, emotion engine, etc.)
            None
        }
        _ => None,
    }
}
