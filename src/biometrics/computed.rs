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
    // Cap active minutes to realistic daily max (don't use cumulative)
    let daily_active = active_minutes.min(120.0); // max 2 hours per day
    let intensity = (avg_hr / hr_max).clamp(0.0, 1.0);
    let trimp = daily_active * intensity * intensity;
    trimp.clamp(0.0, 200.0).round() // max 200 TRIMP per day
}
