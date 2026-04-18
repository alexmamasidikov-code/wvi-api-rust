use super::models::RawMetrics;

/// Normalize raw biometric metrics to 0-100 scores
pub struct MetricNormalizer;

impl MetricNormalizer {
    /// HR Score: closer to resting = better
    pub fn hr_score(heart_rate: f64, resting_hr: f64) -> f64 {
        let delta = (heart_rate - resting_hr).abs();
        (100.0 - delta * 2.5).clamp(0.0, 100.0)
    }

    /// HRV Score: normalized by age
    pub fn hrv_score(hrv: f64, age: u32) -> f64 {
        let age_max = Self::age_based_max_hrv(age);
        ((hrv / age_max) * 100.0).clamp(0.0, 100.0)
    }

    /// Stress Score: inverted (0=calm=100, 100=stressed=0)
    pub fn stress_score(stress: f64) -> f64 {
        (100.0 - stress).clamp(0.0, 100.0)
    }

    /// SpO₂ Score: non-linear scale per master spec clinical tiers
    /// (≥95 normal, 90-94 mild hypoxia, <90 significant hypoxia).
    /// Input is clamped to 70-100 so sensor glitches don't emit zeros.
    pub fn spo2_score(spo2: f64) -> f64 {
        let s = spo2.clamp(70.0, 100.0);
        if s >= 98.0 {
            80.0 + (s - 98.0) * 10.0            // 98-100 → 80-100
        } else if s >= 95.0 {
            50.0 + (s - 95.0) * 10.0            // 95-98 normal → 50-80
        } else if s >= 90.0 {
            20.0 + (s - 90.0) * 6.0             // 90-94 mild hypoxia → 20-50
        } else {
            ((s - 70.0) / 20.0 * 20.0).max(0.0) // <90 significant → 0-20
        }
        .clamp(0.0, 100.0)
    }

    /// Temperature Score: deviation from personal baseline
    pub fn temperature_score(temp: f64, base_temp: f64) -> f64 {
        let delta = (temp - base_temp).abs();
        (100.0 - delta * 40.0).clamp(0.0, 100.0)
    }

    /// Sleep Score: composite of deep%, duration, continuity
    pub fn sleep_score(total_minutes: f64, deep_percent: f64, continuity: f64) -> f64 {
        let total_hours = total_minutes / 60.0;

        let deep_score = if (15.0..=25.0).contains(&deep_percent) {
            100.0
        } else {
            (100.0 - (deep_percent - 20.0).abs() * 5.0).max(0.0)
        };

        let duration_score = if (7.0..=9.0).contains(&total_hours) {
            100.0
        } else {
            (100.0 - (total_hours - 8.0).abs() * 20.0).max(0.0)
        };

        let cont_score = continuity * 100.0;

        deep_score * 0.35 + duration_score * 0.40 + cont_score * 0.25
    }

    /// Activity Score: steps + active minutes + METS bonus
    pub fn activity_score(steps: f64, active_mins: f64, mets: f64) -> f64 {
        let step_ratio = (steps / 10000.0).min(1.0);
        let active_min_ratio = (active_mins / 30.0).min(1.0);
        let mets_bonus = (mets / 8.0).min(1.0) * 20.0;
        (step_ratio * 45.0 + active_min_ratio * 35.0 + mets_bonus).min(100.0)
    }

    /// BP Score: optimal 120/80, each mmHg deviation = -1.5
    pub fn bp_score(systolic: f64, diastolic: f64) -> f64 {
        let deviation = (systolic - 120.0).abs() + (diastolic - 80.0).abs();
        (100.0 - deviation * 1.5).clamp(0.0, 100.0)
    }

    /// PPI Coherence Score: coherence 0-1 → 0-100
    pub fn ppi_score(coherence: f64) -> f64 {
        (coherence * 100.0).clamp(0.0, 100.0)
    }

    /// Age-based max HRV reference values
    fn age_based_max_hrv(age: u32) -> f64 {
        match age {
            0..=29 => 74.0,
            30..=39 => 62.0,
            40..=49 => 52.0,
            50..=59 => 42.0,
            _ => 35.0,
        }
    }

    /// Normalize all metrics from raw data
    pub fn normalize_all(raw: &RawMetrics) -> super::models::MetricScores {
        super::models::MetricScores {
            heart_rate: Self::hr_score(raw.heart_rate, raw.resting_hr),
            hrv: Self::hrv_score(raw.hrv, raw.age),
            stress: Self::stress_score(raw.stress),
            spo2: Self::spo2_score(raw.spo2),
            temperature: Self::temperature_score(raw.temperature, raw.base_temp),
            sleep: Self::sleep_score(raw.total_sleep_minutes, raw.deep_sleep_percent, raw.sleep_continuity),
            activity: Self::activity_score(raw.steps, raw.active_minutes, raw.mets),
            blood_pressure: Self::bp_score(raw.systolic_bp, raw.diastolic_bp),
            ppi_coherence: Self::ppi_score(raw.ppi_coherence),
            emotional_wellbeing: 50.0, // computed separately from emotion history
        }
    }
}
