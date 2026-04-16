/// Estimate BP from HRV and resting HR
/// PTT-based estimation: lower HRV + higher HR → higher BP
pub fn estimate_blood_pressure(hr: f64, hrv: f64) -> (f64, f64) {
    // Systolic: base 120, adjusted by HR deviation from 70 and HRV
    let sys = 120.0 + (hr - 70.0) * 0.5 - (hrv - 50.0) * 0.3;
    let sys = sys.clamp(90.0, 180.0);
    // Diastolic: typically 60-65% of systolic
    let dia = sys * 0.63;
    let dia = dia.clamp(60.0, 120.0);
    (sys.round(), dia.round())
}

/// Estimate VO2 Max from resting HR (Uth et al. formula)
/// VO2max = 15.3 × (HRmax / HRrest)
pub fn estimate_vo2_max(resting_hr: f64, age: f64) -> f64 {
    let hr_max = 220.0 - age;
    let vo2 = 15.3 * (hr_max / resting_hr);
    vo2.clamp(15.0, 75.0)
}

/// Compute cardiac coherence from HRV RMSSD
/// High coherence = rhythmic HRV patterns
pub fn compute_coherence(hrv_rmssd: f64) -> f64 {
    // Simplified: coherence correlates with HRV
    let coherence = (hrv_rmssd / 100.0 * 80.0).clamp(0.0, 100.0);
    coherence
}

/// Compute sleep score from phases (0-100)
pub fn compute_sleep_score(deep_pct: f64, rem_pct: f64, duration_hours: f64, awake_pct: f64) -> f64 {
    // Deep sleep: ideal 15-25%, weight 35%
    let deep_score = if deep_pct >= 15.0 && deep_pct <= 25.0 { 100.0 }
        else if deep_pct >= 10.0 { 70.0 } else { 40.0 };
    // REM: ideal 20-25%, weight 25%
    let rem_score = if rem_pct >= 20.0 && rem_pct <= 25.0 { 100.0 }
        else if rem_pct >= 15.0 { 70.0 } else { 40.0 };
    // Duration: ideal 7-9h, weight 25%
    let dur_score = if duration_hours >= 7.0 && duration_hours <= 9.0 { 100.0 }
        else if duration_hours >= 6.0 { 70.0 } else { 40.0 };
    // Awake: ideal < 5%, weight 15%
    let awake_score = if awake_pct < 5.0 { 100.0 }
        else if awake_pct < 10.0 { 70.0 } else { 40.0 };

    let score: f64 = deep_score * 0.35 + rem_score * 0.25 + dur_score * 0.25 + awake_score * 0.15;
    score.round()
}

/// Compute biological age from metrics
/// Lower HR, higher HRV, higher SpO2, more activity → younger bio age
pub fn compute_bio_age(chronological_age: f64, hr: f64, hrv: f64, spo2: f64, steps: f64, sleep_score: f64) -> f64 {
    let mut bio_age = chronological_age;
    // HRV: each 10ms above 50 = -0.5 year (was -1, too aggressive)
    bio_age -= ((hrv - 50.0) / 10.0 * 0.5).clamp(-3.0, 3.0);
    // HR: each 5bpm below 70 = -0.3 year (was -0.5)
    bio_age -= ((70.0 - hr) / 5.0 * 0.3).clamp(-2.0, 2.0);
    // Activity: 10k steps = -1 year (was -2)
    bio_age -= (steps / 10000.0 * 1.0).clamp(0.0, 2.0);
    // SpO2 penalty
    if spo2 > 0.0 && spo2 < 95.0 { bio_age += 1.5; }
    // Sleep: good sleep = -0.5 year (was -1)
    bio_age -= ((sleep_score - 50.0) / 50.0 * 0.5).clamp(-1.0, 1.0);
    bio_age.clamp(18.0, 120.0).round()
}

/// Compute training load from recent activity
/// TRIMP-based: duration × HR intensity
pub fn compute_training_load(active_minutes: f64, avg_hr: f64, hr_max: f64) -> f64 {
    // Defensive: guard against zero / negative hr_max which would produce NaN / Inf
    let hr_max_safe = if hr_max > 0.0 { hr_max } else { 180.0 };
    let active_safe = active_minutes.max(0.0);
    let avg_hr_safe = avg_hr.max(0.0);

    // Cap active minutes to realistic daily max (don't use cumulative)
    let daily_active = active_safe.min(120.0); // max 2 hours per day
    let intensity = (avg_hr_safe / hr_max_safe).clamp(0.0, 1.0);
    let trimp = daily_active * intensity * intensity;
    trimp.clamp(0.0, 200.0).round() // max 200 TRIMP per day
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Blood Pressure ----
    #[test]
    fn bp_normal_hrv_gives_normal_sys() {
        let (sys, dia) = estimate_blood_pressure(70.0, 55.0);
        assert!(sys >= 115.0 && sys <= 125.0, "expected ~120 sys, got {}", sys);
        assert!(dia >= 70.0 && dia <= 80.0, "expected ~75 dia, got {}", dia);
    }

    #[test]
    fn bp_high_hr_low_hrv_gives_elevated_bp() {
        let (sys, _) = estimate_blood_pressure(100.0, 20.0);
        assert!(sys > 130.0, "elevated HR+low HRV should give sys > 130, got {}", sys);
    }

    #[test]
    fn bp_clamped_at_boundaries() {
        let (sys_hi, dia_hi) = estimate_blood_pressure(200.0, 0.0);
        let (sys_lo, dia_lo) = estimate_blood_pressure(40.0, 150.0);
        assert!(sys_hi <= 180.0 && sys_lo >= 90.0);
        assert!(dia_hi <= 120.0 && dia_lo >= 60.0);
    }

    #[test]
    fn bp_zero_inputs_dont_produce_nan() {
        let (sys, dia) = estimate_blood_pressure(0.0, 0.0);
        assert!(sys.is_finite() && dia.is_finite());
        assert!(sys >= 90.0 && sys <= 180.0);
    }

    // ---- VO2 Max ----
    #[test]
    fn vo2_young_fit_gives_high_score() {
        let vo2 = estimate_vo2_max(55.0, 25.0);
        assert!(vo2 >= 50.0, "young athlete should have VO2 > 50, got {}", vo2);
    }

    #[test]
    fn vo2_older_average_gives_moderate_score() {
        let vo2 = estimate_vo2_max(75.0, 55.0);
        assert!(vo2 >= 25.0 && vo2 <= 40.0, "older moderate should be 25-40, got {}", vo2);
    }

    #[test]
    fn vo2_clamped_to_15_75_range() {
        let vo2_extreme_low = estimate_vo2_max(200.0, 100.0);
        let vo2_extreme_high = estimate_vo2_max(30.0, 10.0);
        assert!(vo2_extreme_low >= 15.0);
        assert!(vo2_extreme_high <= 75.0);
    }

    // ---- Coherence ----
    #[test]
    fn coherence_scales_with_hrv() {
        assert!(compute_coherence(100.0) > compute_coherence(50.0));
        assert!(compute_coherence(50.0) > compute_coherence(10.0));
    }

    #[test]
    fn coherence_clamped_0_100() {
        assert_eq!(compute_coherence(0.0), 0.0);
        let high = compute_coherence(500.0);
        assert!(high <= 100.0);
    }

    // ---- Sleep Score ----
    #[test]
    fn sleep_score_ideal_phases_gives_high() {
        let score = compute_sleep_score(20.0, 22.0, 8.0, 3.0);
        assert!(score >= 95.0, "ideal sleep should be ~100, got {}", score);
    }

    #[test]
    fn sleep_score_poor_gives_low() {
        let score = compute_sleep_score(5.0, 5.0, 4.0, 25.0);
        assert!(score <= 60.0, "poor sleep should be <=60, got {}", score);
    }

    #[test]
    fn sleep_score_all_zeros_doesnt_panic() {
        let score = compute_sleep_score(0.0, 0.0, 0.0, 0.0);
        assert!(score.is_finite());
        assert!(score >= 0.0 && score <= 100.0);
    }

    // ---- Bio Age ----
    #[test]
    fn bio_age_young_healthy_is_lower_than_chronological() {
        let bio = compute_bio_age(35.0, 60.0, 75.0, 98.0, 12000.0, 85.0);
        assert!(bio < 35.0, "healthy biometrics should reduce bio age, got {}", bio);
    }

    #[test]
    fn bio_age_unhealthy_is_higher_than_chronological() {
        let bio = compute_bio_age(35.0, 90.0, 20.0, 93.0, 1000.0, 35.0);
        assert!(bio > 35.0, "unhealthy biometrics should increase bio age, got {}", bio);
    }

    #[test]
    fn bio_age_zero_inputs_doesnt_panic() {
        let bio = compute_bio_age(40.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!(bio.is_finite());
    }

    // ---- Training Load ----
    #[test]
    fn training_load_capped_at_200() {
        let load = compute_training_load(500.0, 180.0, 25.0);
        assert!(load <= 200.0);
    }

    #[test]
    fn training_load_active_minutes_capped_at_120() {
        let load_high = compute_training_load(500.0, 150.0, 180.0);
        let load_ceiling = compute_training_load(120.0, 150.0, 180.0);
        // Going from 120→500 active minutes shouldn't change load (already capped)
        assert_eq!(load_high, load_ceiling);
    }

    #[test]
    fn training_load_zero_inputs_yields_zero() {
        let load = compute_training_load(0.0, 0.0, 180.0);
        assert_eq!(load, 0.0);
    }

    #[test]
    fn training_load_zero_hr_max_doesnt_panic() {
        // Fail-safe: avg_hr / 0 would produce inf; defensive code handles it
        let load = compute_training_load(60.0, 150.0, 0.0);
        assert!(load.is_finite());
        assert!(load >= 0.0 && load <= 200.0);
    }

    #[test]
    fn training_load_negative_inputs_dont_panic() {
        let load = compute_training_load(-50.0, -200.0, -10.0);
        assert!(load.is_finite());
        assert!(load >= 0.0 && load <= 200.0);
    }
}
