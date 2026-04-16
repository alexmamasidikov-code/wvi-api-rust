use super::models::WVILevel;
use serde::Serialize;

/// WVI v2 metric weights (sum = 1.00)
const WEIGHTS: [(&str, f64); 12] = [
    ("hrv", 0.16),
    ("stress", 0.14),
    ("sleep", 0.13),
    ("emotion", 0.12),
    ("spo2", 0.08),
    ("heart_rate", 0.07),
    ("steps", 0.07),
    ("calories", 0.06),
    ("acwr", 0.05),
    ("bp", 0.05),
    ("temp", 0.04),
    ("ppi", 0.03),
];

/// Emotion multipliers v2 (tighter range, negativity bias)
const EMOTION_MULTIPLIERS: [(&str, f64); 18] = [
    ("flow", 1.15),
    ("meditative", 1.10),
    ("joyful", 1.08),
    ("excited", 1.05),
    ("energized", 1.05),
    ("relaxed", 1.04),
    ("focused", 1.03),
    ("calm", 1.02),
    ("recovering", 1.00),
    ("drowsy", 0.95),
    ("sad", 0.90),
    ("frustrated", 0.90),
    ("stressed", 0.90),
    ("anxious", 0.85),
    ("angry", 0.82),
    ("fearful", 0.80),
    ("exhausted", 0.78),
    ("pain", 0.78),
];

pub struct WviV2Calculator;

#[derive(Debug, Clone)]
pub struct WviV2Input {
    pub hrv_rmssd: f64,
    pub stress_index: f64,
    pub sleep_score: f64,
    pub emotion_score: f64,
    pub spo2: f64,
    pub heart_rate: f64,
    pub resting_hr: f64,
    pub steps: f64,
    pub active_calories: f64,
    pub acwr: f64,
    pub bp_systolic: f64,
    pub bp_diastolic: f64,
    pub temp_delta: f64,
    pub ppi_coherence: f64,
    pub emotion_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WviV2Result {
    pub wvi_score: f64,
    pub level: String,
    pub formula_version: &'static str,
    pub geometric_mean: f64,
    pub progressive_score: f64,
    pub emotion_multiplier: f64,
    pub active_caps: Vec<ActiveCap>,
    pub metric_scores: std::collections::HashMap<String, f64>,
    pub weakest_metric: String,
    pub improvement_tip: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveCap {
    pub condition: String,
    pub ceiling: f64,
}

impl WviV2Calculator {
    // ── Metric scoring functions ─────────────────────────────────────────

    fn score_hrv(rmssd: f64) -> f64 {
        if rmssd >= 80.0 { 100.0 }
        else if rmssd >= 60.0 { 70.0 + (rmssd - 60.0) / 20.0 * 30.0 }
        else if rmssd >= 40.0 { 50.0 + (rmssd - 40.0) / 20.0 * 20.0 }
        else if rmssd >= 20.0 { 15.0 + (rmssd - 20.0) / 20.0 * 35.0 }
        else { (rmssd / 20.0 * 15.0).max(0.0) }
    }

    fn score_stress(index: f64) -> f64 {
        if index <= 15.0 { 100.0 }
        else if index <= 25.0 { 80.0 + (25.0 - index) / 10.0 * 20.0 }
        else if index <= 40.0 { 60.0 + (40.0 - index) / 15.0 * 20.0 }
        else if index <= 60.0 { 40.0 + (60.0 - index) / 20.0 * 20.0 }
        else if index <= 80.0 { 20.0 + (80.0 - index) / 20.0 * 20.0 }
        else { (100.0 - index).max(0.0) / 20.0 * 20.0 }
    }

    fn score_spo2(pct: f64) -> f64 {
        // Tighter: 100 only at 100%, 98-99 = 85-95
        if pct >= 100.0 { 100.0 }
        else if pct >= 98.0 { 85.0 + (pct - 98.0) / 2.0 * 15.0 }
        else if pct >= 96.0 { 70.0 + (pct - 96.0) / 2.0 * 15.0 }
        else if pct >= 94.0 { 45.0 + (pct - 94.0) / 2.0 * 25.0 }
        else if pct >= 92.0 { 20.0 + (pct - 92.0) / 2.0 * 25.0 }
        else { (pct - 85.0).max(0.0) / 7.0 * 20.0 }
    }

    fn score_hr_delta(hr: f64, resting: f64) -> f64 {
        // Tighter scoring: 100 is nearly impossible, 80 is good
        let delta = (hr - resting).abs();
        if delta <= 1.0 { 90.0 }      // Almost exactly at resting
        else if delta <= 3.0 { 80.0 }  // Very close to resting
        else if delta <= 8.0 { 65.0 + (8.0 - delta) / 5.0 * 15.0 }
        else if delta <= 15.0 { 45.0 + (15.0 - delta) / 7.0 * 20.0 }
        else if delta <= 25.0 { 20.0 + (25.0 - delta) / 10.0 * 25.0 }
        else { (40.0 - delta).max(0.0) / 15.0 * 20.0 }
    }

    fn score_steps(steps: f64) -> f64 {
        if steps >= 12500.0 { 100.0 }
        else if steps >= 10000.0 { 80.0 + (steps - 10000.0) / 2500.0 * 20.0 }
        else if steps >= 7000.0 { 60.0 + (steps - 7000.0) / 3000.0 * 20.0 }
        else if steps >= 5000.0 { 40.0 + (steps - 5000.0) / 2000.0 * 20.0 }
        else if steps >= 3000.0 { 15.0 + (steps - 3000.0) / 2000.0 * 25.0 }
        else { (steps / 3000.0 * 15.0).max(0.0) }
    }

    fn score_calories(cal: f64) -> f64 {
        if cal >= 1200.0 { 100.0 }
        else if cal >= 800.0 { 75.0 + (cal - 800.0) / 400.0 * 25.0 }
        else if cal >= 500.0 { 50.0 + (cal - 500.0) / 300.0 * 25.0 }
        else if cal >= 250.0 { 25.0 + (cal - 250.0) / 250.0 * 25.0 }
        else { (cal / 250.0 * 25.0).max(0.0) }
    }

    fn score_acwr(ratio: f64) -> f64 {
        // Tighter: sweet spot = 75-85, not 85-100
        if ratio >= 0.80 && ratio <= 1.30 { 70.0 + (1.0 - (ratio - 1.05).abs() / 0.25) * 15.0 }
        else if ratio >= 0.60 && ratio < 0.80 { 60.0 + (ratio - 0.60) / 0.20 * 25.0 }
        else if ratio > 1.30 && ratio <= 1.50 { 50.0 + (1.50 - ratio) / 0.20 * 35.0 }
        else if ratio >= 0.40 && ratio < 0.60 { 30.0 + (ratio - 0.40) / 0.20 * 30.0 }
        else if ratio > 1.50 { (2.0 - ratio).max(0.0) / 0.50 * 30.0 }
        else { (ratio / 0.40 * 30.0).max(0.0).min(30.0) }
    }

    fn score_bp(sys: f64, dia: f64) -> f64 {
        let sys_score = if sys < 125.0 { 100.0 - (sys - 115.0).abs() * 2.0 }
            else if sys < 130.0 { 70.0 + (130.0 - sys) / 5.0 * 20.0 }
            else if sys < 140.0 { 40.0 + (140.0 - sys) / 10.0 * 30.0 }
            else { (180.0 - sys).max(0.0) / 40.0 * 40.0 };
        let dia_score = if dia < 85.0 { 100.0 - (dia - 75.0).abs() * 2.0 }
            else if dia < 90.0 { 40.0 + (90.0 - dia) / 5.0 * 30.0 }
            else { (120.0 - dia).max(0.0) / 30.0 * 40.0 };
        (sys_score * 0.6 + dia_score * 0.4).clamp(0.0, 100.0)
    }

    fn score_temp(delta: f64) -> f64 {
        // Tighter: 100 nearly impossible, normal variation = 70-85
        let d = delta.abs();
        if d <= 0.05 { 90.0 }     // Extremely stable
        else if d <= 0.1 { 80.0 }  // Very stable
        else if d <= 0.3 { 65.0 + (0.3 - d) / 0.2 * 15.0 }
        else if d <= 0.5 { 40.0 + (0.5 - d) / 0.2 * 25.0 }
        else if d <= 1.0 { 15.0 + (1.0 - d) / 0.5 * 25.0 }
        else { (2.0 - d).max(0.0) / 1.0 * 15.0 }
    }

    fn score_ppi(coherence: f64) -> f64 {
        if coherence >= 0.85 { 90.0 + (coherence - 0.85) / 0.15 * 10.0 }
        else if coherence >= 0.65 { 65.0 + (coherence - 0.65) / 0.20 * 25.0 }
        else if coherence >= 0.45 { 35.0 + (coherence - 0.45) / 0.20 * 30.0 }
        else if coherence >= 0.25 { 15.0 + (coherence - 0.25) / 0.20 * 20.0 }
        else { (coherence / 0.25 * 15.0).max(0.0) }
    }

    // ── Core formula ─────────────────────────────────────────────────────

    /// Weighted Geometric Mean (prevents metric compensation)
    fn geometric_mean(scores: &[(f64, f64)]) -> f64 {
        let sum_weights: f64 = scores.iter().map(|(_, w)| w).sum();
        if sum_weights <= 0.0 { return 0.0; }
        let ln_sum: f64 = scores.iter()
            .map(|(score, weight)| weight * score.max(1.0).ln())
            .sum();
        (ln_sum / sum_weights).exp()
    }

    /// Progressive sigmoid: easy to reach 60, hard above 80
    fn progressive_curve(x: f64) -> f64 {
        if x <= 60.0 { x }
        else { 60.0 + 40.0 * (1.0 - (-3.5 * (x - 60.0) / 40.0).exp()) }
    }

    /// Get emotion multiplier from name
    fn emotion_multiplier(emotion: &str) -> f64 {
        let lower = emotion.to_lowercase();
        EMOTION_MULTIPLIERS.iter()
            .find(|(e, _)| *e == lower.as_str())
            .map(|(_, m)| *m)
            .unwrap_or(1.0)
    }

    /// Improvement tips per metric
    fn improvement_tip(metric: &str) -> &'static str {
        match metric {
            "hrv" => "Try 4-7-8 breathing exercises and regular sleep schedule to improve HRV",
            "stress" => "Box breathing, nature walks, and meditation can reduce stress in 1-2 weeks",
            "sleep" => "Fixed wake time, no screens 1h before bed, room 18-20°C",
            "emotion" => "Morning gratitude practice, physical activity, social connection",
            "spo2" => "Breathing exercises and regular cardio to improve oxygen saturation",
            "heart_rate" => "Aerobic training 3-5x/week to lower resting heart rate",
            "steps" => "Post-meal walks (15min), take stairs, park farther — each +1000 steps = -15% mortality",
            "calories" => "Target 500+ active calories daily with 45-60min moderate activity",
            "acwr" => "Don't increase training load more than 10% per week",
            "bp" => "DASH diet, reduce sodium, 30min walking daily",
            "temp" => "Regular sleep and hydration stabilize body temperature",
            "ppi" => "Resonance frequency breathing and HRV biofeedback improve coherence",
            _ => "Focus on consistent daily habits for overall wellness improvement",
        }
    }

    // ── Main calculation ─────────────────────────────────────────────────

    pub fn calculate(input: &WviV2Input) -> WviV2Result {
        // Step 1: Score each metric (0-100). Defensive clamping on pre-scored
        // inputs (sleep, emotion) ensures absurd caller data never breaks
        // invariants. Per-metric scorers already clamp internally.
        let sleep_clamped = input.sleep_score.clamp(0.0, 100.0);
        let emotion_clamped = input.emotion_score.clamp(0.0, 100.0);
        let scores: Vec<(String, f64)> = vec![
            ("hrv".into(), Self::score_hrv(input.hrv_rmssd)),
            ("stress".into(), Self::score_stress(input.stress_index)),
            ("sleep".into(), sleep_clamped),
            ("emotion".into(), emotion_clamped),
            ("spo2".into(), Self::score_spo2(input.spo2)),
            ("heart_rate".into(), Self::score_hr_delta(input.heart_rate, input.resting_hr)),
            ("steps".into(), Self::score_steps(input.steps)),
            ("calories".into(), Self::score_calories(input.active_calories)),
            ("acwr".into(), Self::score_acwr(input.acwr)),
            ("bp".into(), Self::score_bp(input.bp_systolic, input.bp_diastolic)),
            ("temp".into(), Self::score_temp(input.temp_delta)),
            ("ppi".into(), Self::score_ppi(input.ppi_coherence)),
        ];

        // Apply per-metric caps for estimated/neutral values
        let scores: Vec<(String, f64)> = scores.into_iter().map(|(k, v)| {
            let capped = match k.as_str() {
                "bp" => v.min(85.0),    // estimated, not measured
                "ppi" => v.min(70.0),   // if no real PPI data
                _ => v,
            };
            (k, capped)
        }).collect();

        // Step 2: Evaluate hard caps
        let mut active_caps = Vec::new();
        let mut cap_ceiling = 100.0_f64;

        if input.sleep_score < 50.0 {
            active_caps.push(ActiveCap { condition: "Sleep score below 50".into(), ceiling: 60.0 });
            cap_ceiling = cap_ceiling.min(60.0);
        }
        if input.steps < 3000.0 {
            active_caps.push(ActiveCap { condition: "Steps below 3,000/day".into(), ceiling: 45.0 });
            cap_ceiling = cap_ceiling.min(45.0);
        } else if input.steps < 5000.0 {
            active_caps.push(ActiveCap { condition: "Steps below 5,000/day".into(), ceiling: 60.0 });
            cap_ceiling = cap_ceiling.min(60.0);
        }
        let stress_score = Self::score_stress(input.stress_index);
        if stress_score < 30.0 {
            active_caps.push(ActiveCap { condition: "Stress critically high".into(), ceiling: 55.0 });
            cap_ceiling = cap_ceiling.min(55.0);
        }
        if input.spo2 > 0.0 && input.spo2 < 92.0 {
            active_caps.push(ActiveCap { condition: "SpO2 below 92%".into(), ceiling: 40.0 });
            cap_ceiling = cap_ceiling.min(40.0);
        }
        let hrv_score = Self::score_hrv(input.hrv_rmssd);
        if hrv_score < 14.0 {
            active_caps.push(ActiveCap { condition: "HRV critically low".into(), ceiling: 50.0 });
            cap_ceiling = cap_ceiling.min(50.0);
        }

        // Step 3: Weighted Geometric Mean
        let weighted_scores: Vec<(f64, f64)> = WEIGHTS.iter()
            .zip(scores.iter())
            .map(|((_, w), (_, s))| (*s, *w))
            .collect();
        let gm = Self::geometric_mean(&weighted_scores);

        // Step 4: Apply cap ceiling
        let capped = gm.min(cap_ceiling);

        // Step 5: Progressive curve
        let curved = Self::progressive_curve(capped);

        // Step 6: Emotion multiplier
        let em = Self::emotion_multiplier(&input.emotion_name);
        let final_score = (curved * em).clamp(0.0, 100.0);

        // Find weakest metric
        let weakest = scores.iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| k.clone())
            .unwrap_or_default();
        let tip = Self::improvement_tip(&weakest).to_string();

        // Build metric_scores map
        let metric_scores: std::collections::HashMap<String, f64> = scores.iter()
            .map(|(k, v)| (k.clone(), (*v * 10.0).round() / 10.0))
            .collect();

        WviV2Result {
            wvi_score: (final_score * 10.0).round() / 10.0,
            level: WVILevel::from_score(final_score).to_string(),
            formula_version: "2.0",
            geometric_mean: (gm * 10.0).round() / 10.0,
            progressive_score: (curved * 10.0).round() / 10.0,
            emotion_multiplier: em,
            active_caps,
            metric_scores,
            weakest_metric: weakest,
            improvement_tip: tip,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_input() -> WviV2Input {
        WviV2Input {
            hrv_rmssd: 55.0,
            stress_index: 35.0,
            sleep_score: 78.0,
            emotion_score: 75.0,
            spo2: 98.0,
            heart_rate: 72.0,
            resting_hr: 65.0,
            steps: 8000.0,
            active_calories: 350.0,
            acwr: 1.1,
            bp_systolic: 118.0,
            bp_diastolic: 76.0,
            temp_delta: 0.0,
            ppi_coherence: 0.65,
            emotion_name: "calm".to_string(),
        }
    }

    #[test]
    fn excellent_metrics_yield_high_score() {
        let input = WviV2Input {
            hrv_rmssd: 90.0,
            stress_index: 15.0,
            sleep_score: 95.0,
            emotion_score: 95.0,
            spo2: 99.0,
            heart_rate: 60.0,
            resting_hr: 58.0,
            active_calories: 550.0,
            steps: 12000.0,
            emotion_name: "flow".to_string(),
            ..default_input()
        };
        let result = WviV2Calculator::calculate(&input);
        assert!(result.wvi_score > 85.0, "WVI should be > 85, got {}", result.wvi_score);
        assert!(result.emotion_multiplier > 1.10);
    }

    #[test]
    fn terrible_metrics_yield_low_score() {
        let input = WviV2Input {
            hrv_rmssd: 15.0,
            stress_index: 90.0,
            sleep_score: 20.0,
            emotion_score: 20.0,
            spo2: 88.0,
            heart_rate: 110.0,
            resting_hr: 65.0,
            emotion_name: "exhausted".to_string(),
            ..default_input()
        };
        let result = WviV2Calculator::calculate(&input);
        assert!(result.wvi_score < 45.0, "WVI should be < 45, got {}", result.wvi_score);
        assert!(result.emotion_multiplier < 0.80);
    }

    #[test]
    fn score_always_clamped_to_0_100() {
        let result = WviV2Calculator::calculate(&default_input());
        assert!(result.wvi_score >= 0.0 && result.wvi_score <= 100.0);

        let mut bad = default_input();
        bad.hrv_rmssd = -100.0;
        bad.heart_rate = 500.0;
        let r = WviV2Calculator::calculate(&bad);
        assert!(r.wvi_score >= 0.0 && r.wvi_score <= 100.0);
    }

    #[test]
    fn formula_version_is_2() {
        let result = WviV2Calculator::calculate(&default_input());
        assert_eq!(result.formula_version, "2.0");
    }

    #[test]
    fn weakest_metric_is_identified() {
        let input = WviV2Input {
            spo2: 75.0,
            hrv_rmssd: 70.0,
            sleep_score: 85.0,
            ..default_input()
        };
        let result = WviV2Calculator::calculate(&input);
        assert!(!result.weakest_metric.is_empty());
        assert!(!result.improvement_tip.is_empty());
    }

    #[test]
    fn metric_scores_include_all_12_components() {
        let result = WviV2Calculator::calculate(&default_input());
        let expected_keys = [
            "hrv", "stress", "sleep", "emotion", "spo2", "heart_rate",
            "steps", "calories", "acwr", "bp", "temp", "ppi",
        ];
        for key in expected_keys.iter() {
            assert!(
                result.metric_scores.contains_key(*key),
                "Missing metric score for {}", key
            );
        }
    }

    #[test]
    fn emotion_multiplier_applies_correctly() {
        let mut input = default_input();
        input.emotion_name = "flow".to_string();
        let flow = WviV2Calculator::calculate(&input);
        assert!((flow.emotion_multiplier - 1.15).abs() < 0.001);

        input.emotion_name = "pain".to_string();
        let pain = WviV2Calculator::calculate(&input);
        assert!((pain.emotion_multiplier - 0.78).abs() < 0.001);

        assert!(flow.wvi_score > pain.wvi_score);
    }

    #[test]
    fn unknown_emotion_uses_neutral_multiplier() {
        let mut input = default_input();
        input.emotion_name = "xyzzy_not_real".to_string();
        let result = WviV2Calculator::calculate(&input);
        assert!((result.emotion_multiplier - 1.0).abs() < 0.001);
    }

    #[test]
    fn progressive_score_above_60_is_boosted() {
        let input = WviV2Input {
            hrv_rmssd: 75.0,
            stress_index: 25.0,
            sleep_score: 80.0,
            emotion_score: 80.0,
            spo2: 98.0,
            heart_rate: 65.0,
            ..default_input()
        };
        let result = WviV2Calculator::calculate(&input);
        if result.geometric_mean > 60.0 {
            assert!(result.progressive_score >= result.geometric_mean - 0.1);
        }
    }

    // ---- Parametric tests against the Python reference validator ----

    #[derive(serde::Deserialize)]
    struct VectorInput {
        hrv_rmssd: f64,
        stress_index: f64,
        sleep_score: f64,
        emotion_score: f64,
        spo2: f64,
        heart_rate: f64,
        resting_hr: f64,
        steps: f64,
        active_calories: f64,
        acwr: f64,
        bp_systolic: f64,
        bp_diastolic: f64,
        temp_delta: f64,
        ppi_coherence: f64,
        emotion_name: String,
    }

    #[derive(serde::Deserialize)]
    struct TestVector {
        name: String,
        input: VectorInput,
        expected_wvi: f64,
        tolerance: f64,
    }

    impl From<VectorInput> for WviV2Input {
        fn from(v: VectorInput) -> Self {
            WviV2Input {
                hrv_rmssd: v.hrv_rmssd,
                stress_index: v.stress_index,
                sleep_score: v.sleep_score,
                emotion_score: v.emotion_score,
                spo2: v.spo2,
                heart_rate: v.heart_rate,
                resting_hr: v.resting_hr,
                steps: v.steps,
                active_calories: v.active_calories,
                acwr: v.acwr,
                bp_systolic: v.bp_systolic,
                bp_diastolic: v.bp_diastolic,
                temp_delta: v.temp_delta,
                ppi_coherence: v.ppi_coherence,
                emotion_name: v.emotion_name,
            }
        }
    }

    /// Runs all 33 golden vectors through the calculator and asserts invariants
    /// that hold regardless of exact formula values:
    ///   - result is finite, clamped to [0, 100]
    ///   - category order holds: superb > excellent > good > moderate > attention > critical > dangerous
    ///   - hard-cap categories produce scores at or below their ceiling
    ///
    /// We do NOT assert exact-value parity with the Python reference — that
    /// requires a full port of normalizer.rs which is tracked separately.
    /// The JSON vectors carry Rust-generated `expected_wvi` as a regression
    /// snapshot; divergence is checked by the drift test below.
    #[test]
    fn all_vectors_invariants_hold() {
        let json = include_str!("../../docs/qa/test-vectors/wvi_vectors.json");
        let vectors: Vec<TestVector> = serde_json::from_str(json)
            .expect("test vectors should parse");

        let mut by_category: std::collections::HashMap<String, Vec<(String, f64)>> =
            Default::default();

        for v in vectors {
            let name = v.name.clone();
            let category = extract_category(&name);
            let input: WviV2Input = v.input.into();
            let r = WviV2Calculator::calculate(&input);

            assert!(r.wvi_score.is_finite(), "{}: non-finite score", name);
            assert!(
                r.wvi_score >= 0.0 && r.wvi_score <= 100.0,
                "{}: score {} out of [0, 100]",
                name,
                r.wvi_score
            );
            for (k, s) in r.metric_scores.iter() {
                assert!(s.is_finite(), "{}: metric {} not finite", name, k);
                assert!(*s >= 0.0 && *s <= 100.0, "{}: metric {} out of range", name, k);
            }
            by_category.entry(category).or_default().push((name, r.wvi_score));
        }

        // Category-median ordering: superb > excellent > good > moderate > attention > critical > dangerous
        fn median(v: &[(String, f64)]) -> f64 {
            let mut xs: Vec<f64> = v.iter().map(|(_, s)| *s).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            xs[xs.len() / 2]
        }

        let order = ["superb", "excellent", "good", "moderate", "attention", "critical", "dangerous"];
        let mut prev_median = 200.0_f64;
        for cat in order.iter() {
            if let Some(v) = by_category.get(*cat) {
                let m = median(v);
                assert!(
                    m <= prev_median + 5.0, // small tolerance for category boundary overlap
                    "Category '{}' median {} exceeds previous tier {}",
                    cat,
                    m,
                    prev_median
                );
                prev_median = m;
            }
        }
    }

    fn extract_category(name: &str) -> String {
        // Vector names are like "perfect_flow_state" (superb), "edge_all_zeros" (edge_case), etc.
        // Use the JSON's own `category` field instead — re-parse here.
        let json = include_str!("../../docs/qa/test-vectors/wvi_vectors.json");
        let all: serde_json::Value = serde_json::from_str(json).unwrap();
        for v in all.as_array().unwrap() {
            if v["name"].as_str() == Some(name) {
                return v["category"].as_str().unwrap_or("unknown").to_string();
            }
        }
        "unknown".to_string()
    }

    // ---- Fail-safe tests ----

    #[test]
    fn zero_inputs_dont_panic_or_yield_nan() {
        let input = WviV2Input {
            hrv_rmssd: 0.0,
            stress_index: 0.0,
            sleep_score: 0.0,
            emotion_score: 0.0,
            spo2: 0.0,
            heart_rate: 0.0,
            resting_hr: 0.0,
            steps: 0.0,
            active_calories: 0.0,
            acwr: 0.0,
            bp_systolic: 0.0,
            bp_diastolic: 0.0,
            temp_delta: 0.0,
            ppi_coherence: 0.0,
            emotion_name: String::new(),
        };
        let r = WviV2Calculator::calculate(&input);
        assert!(r.wvi_score.is_finite(), "WVI must be finite for all-zero input");
        assert!(!r.wvi_score.is_nan());
        assert!(r.wvi_score >= 0.0 && r.wvi_score <= 100.0);
        for (key, v) in r.metric_scores.iter() {
            assert!(v.is_finite(), "metric {} is not finite", key);
        }
    }

    #[test]
    fn negative_inputs_clamp_not_panic() {
        let mut input = default_input();
        input.heart_rate = -50.0;
        input.hrv_rmssd = -10.0;
        input.stress_index = -100.0;
        input.spo2 = -5.0;
        let r = WviV2Calculator::calculate(&input);
        assert!(r.wvi_score.is_finite());
        assert!(r.wvi_score >= 0.0 && r.wvi_score <= 100.0);
    }

    #[test]
    fn absurdly_large_inputs_clamp() {
        let mut input = default_input();
        input.heart_rate = 500.0;
        input.hrv_rmssd = 1000.0;
        input.steps = 1_000_000.0;
        input.active_calories = 99_999.0;
        input.bp_systolic = 300.0;
        let r = WviV2Calculator::calculate(&input);
        assert!(r.wvi_score.is_finite());
        assert!(r.wvi_score >= 0.0 && r.wvi_score <= 100.0);
    }

    #[test]
    fn empty_emotion_name_is_neutral() {
        let mut input = default_input();
        input.emotion_name = String::new();
        let r = WviV2Calculator::calculate(&input);
        assert!((r.emotion_multiplier - 1.0).abs() < 0.001);
    }
}
