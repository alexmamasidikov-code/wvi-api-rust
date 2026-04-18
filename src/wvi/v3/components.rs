//! WVI v3 — 18 pure component functions.
//!
//! Each function takes a user-level input (current value + optional
//! personal Baseline) and returns a 0..100 score. Callers (the
//! aggregator) weight + blend these per-profile. No I/O, no mutability —
//! pure math so tests stay trivial.

use crate::sensitivity::types::Baseline;

pub fn hrv_personal_score(hrv: f64, b: &Baseline) -> f64 {
    if b.std < 1e-6 {
        return 50.0;
    }
    let z = (hrv - b.mean) / b.std;
    (50.0 + z * 15.0).clamp(0.0, 100.0)
}

pub fn signal_burden(penalty_total: f64) -> f64 {
    (100.0 - penalty_total).clamp(0.0, 100.0)
}

pub fn recovery_momentum(avg_7d: f64, avg_30d: f64) -> f64 {
    (50.0 + (avg_7d - avg_30d) * 5.0).clamp(0.0, 100.0)
}

pub fn sleep_composite(minutes: f64, deep_pct: f64, continuity: f64, b: &Baseline) -> f64 {
    let hours = minutes / 60.0;
    let deep = if (15.0..=25.0).contains(&deep_pct) {
        100.0
    } else {
        (100.0 - (deep_pct - 20.0).abs() * 5.0).max(0.0)
    };
    let dur = if (7.0..=9.0).contains(&hours) {
        100.0
    } else {
        (100.0 - (hours - 8.0).abs() * 20.0).max(0.0)
    };
    let cont = continuity * 100.0;
    let base = deep * 0.35 + dur * 0.40 + cont * 0.25;
    let z = if b.std > 0.0 { (base - b.mean) / b.std } else { 0.0 };
    (base + z * 3.0).clamp(0.0, 100.0)
}

pub fn circadian_alignment(recent_24h: &[f64], circadian_mean: &[f64; 24]) -> f64 {
    if recent_24h.len() < 24 {
        return 50.0;
    }
    let mut diffs: f64 = 0.0;
    for h in 0..24 {
        diffs += (recent_24h[h] - circadian_mean[h]).abs();
    }
    let avg_diff = diffs / 24.0;
    (100.0 - avg_diff * 2.0).clamp(0.0, 100.0)
}

pub fn activity_personal_score(steps: f64, active_mins: f64, b: &Baseline) -> f64 {
    let base = ((steps / 10000.0).min(1.0) * 60.0 + (active_mins / 30.0).min(1.0) * 40.0)
        .clamp(0.0, 100.0);
    if b.std < 1e-6 {
        return base;
    }
    let z = (steps - b.mean) / b.std;
    (base + z * 5.0).clamp(0.0, 100.0)
}

pub fn metabolic_efficiency(avg_hr_active: f64, avg_hr_rest: f64, recovery_sec: f64) -> f64 {
    let ratio = if avg_hr_rest > 0.0 {
        avg_hr_active / avg_hr_rest
    } else {
        1.5
    };
    let recovery_score = (120.0 / recovery_sec.max(1.0) * 50.0).clamp(0.0, 50.0);
    let ratio_score = ((2.0 - ratio).max(0.0) * 50.0).clamp(0.0, 50.0);
    (ratio_score + recovery_score).clamp(0.0, 100.0)
}

pub fn stress_personal(stress: f64, b: &Baseline) -> f64 {
    let inverted = 100.0 - stress;
    if b.std < 1e-6 {
        return inverted.clamp(0.0, 100.0);
    }
    let z = (b.mean - stress) / b.std;
    (50.0 + z * 15.0).clamp(0.0, 100.0)
}

pub fn breathing_rate_rest(br: f64, b: &Baseline) -> f64 {
    if b.std < 1e-6 {
        return 50.0;
    }
    let deviation = (br - b.mean).abs() / b.std;
    (100.0 - deviation * 20.0).clamp(0.0, 100.0)
}

pub fn coherence_personal(coherence: f64, b: &Baseline) -> f64 {
    if b.std < 1e-6 {
        return (coherence * 100.0).clamp(0.0, 100.0);
    }
    let z = (coherence - b.mean) / b.std;
    (50.0 + z * 15.0).clamp(0.0, 100.0)
}

pub fn immune_proxy(temp_stability: f64, sleep_quality: f64, hrv_stability: f64) -> f64 {
    (0.35 * temp_stability + 0.35 * sleep_quality + 0.30 * hrv_stability).clamp(0.0, 100.0)
}

pub fn intraday_stability(values_24h: &[f64]) -> f64 {
    if values_24h.is_empty() {
        return 50.0;
    }
    let mean = values_24h.iter().sum::<f64>() / values_24h.len() as f64;
    if mean.abs() < 1e-6 {
        return 50.0;
    }
    let variance =
        values_24h.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values_24h.len() as f64;
    let sd = variance.sqrt();
    let coeff = sd / mean.abs();
    (100.0 - coeff * 500.0).clamp(0.0, 100.0)
}

pub fn emotion_agility(avg_recovery_sec: f64) -> f64 {
    (100.0 - avg_recovery_sec / 60.0).clamp(0.0, 100.0)
}

pub fn emotion_range(hull_area: f64) -> f64 {
    (hull_area / 4.0 * 100.0).clamp(0.0, 100.0)
}

pub fn emotion_anchors(top_3_dwell: &[f64]) -> f64 {
    if top_3_dwell.is_empty() {
        return 50.0;
    }
    let top = top_3_dwell[0];
    if top > 0.7 {
        (100.0 - (top - 0.7) * 150.0).max(0.0)
    } else {
        70.0 + top * 30.0
    }
}

pub fn emotion_regulation(arousal_decay_rate: f64) -> f64 {
    (arousal_decay_rate * 50.0).clamp(0.0, 100.0)
}

pub fn emotion_diversity(entropy: f64, log_18: f64) -> f64 {
    if log_18 <= 0.0 {
        return 0.0;
    }
    (entropy / log_18 * 100.0).clamp(0.0, 100.0)
}

pub fn emotion_contagion(correlation_abs: f64) -> f64 {
    ((1.0 - correlation_abs) * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitivity::types::Baseline;

    fn bl(m: f64, s: f64) -> Baseline {
        Baseline {
            mean: m,
            std: s,
            p10: m - s,
            p90: m + s,
            sample_count: 100,
            locked: true,
        }
    }

    #[test]
    fn hrv_at_baseline_is_50() {
        assert!((hrv_personal_score(50.0, &bl(50.0, 5.0)) - 50.0).abs() < 0.01);
    }

    #[test]
    fn hrv_one_sigma_up_is_65() {
        assert!((hrv_personal_score(55.0, &bl(50.0, 5.0)) - 65.0).abs() < 0.01);
    }

    #[test]
    fn signal_burden_zero_is_100() {
        assert_eq!(signal_burden(0.0), 100.0);
    }

    #[test]
    fn momentum_flat_is_50() {
        assert_eq!(recovery_momentum(70.0, 70.0), 50.0);
    }

    #[test]
    fn momentum_improving() {
        let m = recovery_momentum(75.0, 70.0);
        assert!(m > 60.0);
    }

    #[test]
    fn emotion_diversity_uniform_is_100() {
        let log_18 = (18.0_f64).ln();
        assert_eq!(emotion_diversity(log_18, log_18), 100.0);
    }

    #[test]
    fn emotion_contagion_zero_corr_is_100() {
        assert_eq!(emotion_contagion(0.0), 100.0);
    }

    #[test]
    fn emotion_contagion_full_corr_is_0() {
        assert_eq!(emotion_contagion(1.0), 0.0);
    }

    #[test]
    fn intraday_stability_constant_is_100() {
        let vals = vec![70.0; 24];
        assert!(intraday_stability(&vals) > 95.0);
    }

    #[test]
    fn immune_proxy_all_100_is_100() {
        assert!((immune_proxy(100.0, 100.0, 100.0) - 100.0).abs() < 0.01);
    }
}
