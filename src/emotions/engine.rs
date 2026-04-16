use super::models::*;
use chrono::Utc;

/// Fuzzy logic emotion detection engine — 18 emotions
pub struct EmotionEngine;

impl EmotionEngine {
    /// Sigmoid: smooth 0→1 transition around midpoint
    fn sigmoid(x: f64, mid: f64, k: f64) -> f64 {
        1.0 / (1.0 + (-k * (x - mid)).exp())
    }

    /// Inverse sigmoid: smooth 1→0
    fn sigmoid_inv(x: f64, mid: f64, k: f64) -> f64 {
        1.0 / (1.0 + (k * (x - mid)).exp())
    }

    /// Bell curve: max at center, decay both sides
    fn bell(x: f64, center: f64, width: f64) -> f64 {
        (-(x - center).powi(2) / (2.0 * width.powi(2))).exp()
    }

    /// Detect emotion from biometric signals
    pub fn detect(
        heart_rate: f64, resting_hr: f64, hrv: f64, stress: f64,
        spo2: f64, temperature: f64, base_temp: f64,
        systolic_bp: f64, ppi_coherence: f64, ppi_rmssd: f64,
        sleep_score: f64, activity_score: f64,
        hrv_trend: f64, // -1=falling, 0=stable, 1=rising
        prev_emotion: Option<EmotionState>, elapsed_secs: f64,
    ) -> EmotionResult {
        let delta_hr = heart_rate - resting_hr;
        let temp_delta = temperature - base_temp;
        let mut candidates = Vec::with_capacity(18);

        // ANGRY
        {
            let s = Self::sigmoid(stress, 65.0, 0.15)
                * Self::sigmoid(delta_hr, 22.0, 0.12)
                * Self::sigmoid_inv(hrv, 38.0, 0.10)
                * Self::sigmoid(systolic_bp, 130.0, 0.08)
                * Self::sigmoid_inv(ppi_coherence, 0.35, 8.0)
                * Self::sigmoid(temp_delta, 0.2, 5.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Angry, score: s, weight: 1.0 });
        }

        // ANXIOUS
        {
            let s = Self::sigmoid(stress, 68.0, 0.12)
                * Self::sigmoid_inv(hrv, 32.0, 0.10)
                * Self::sigmoid(delta_hr, 12.0, 0.10)
                * Self::sigmoid_inv(ppi_coherence, 0.28, 8.0)
                * Self::sigmoid_inv(spo2, 97.5, 2.0)
                * Self::sigmoid_inv(systolic_bp, 132.0, 0.05);
            candidates.push(EmotionCandidate { emotion: EmotionState::Anxious, score: s, weight: 0.95 });
        }

        // STRESSED
        {
            let s = Self::sigmoid(stress, 48.0, 0.10)
                * Self::sigmoid_inv(hrv, 52.0, 0.08)
                * Self::sigmoid(delta_hr, 6.0, 0.12);
            candidates.push(EmotionCandidate { emotion: EmotionState::Stressed, score: s, weight: 0.85 });
        }

        // SAD
        {
            let s = Self::sigmoid_inv(hrv, 47.0, 0.08)
                * Self::sigmoid_inv(delta_hr, 6.0, 0.15)
                * Self::bell(stress, 40.0, 20.0)
                * Self::sigmoid_inv(activity_score, 35.0, 0.08)
                * Self::sigmoid_inv(sleep_score, 55.0, 0.06)
                * Self::sigmoid_inv(ppi_coherence, 0.42, 6.0)
                * Self::sigmoid_inv(temp_delta, 0.1, 5.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Sad, score: s, weight: 0.80 });
        }

        // EXHAUSTED
        {
            let s = Self::sigmoid_inv(sleep_score, 42.0, 0.08)
                * Self::sigmoid(stress, 32.0, 0.08)
                * Self::sigmoid_inv(hrv, 42.0, 0.08)
                * Self::sigmoid_inv(spo2, 96.5, 1.5)
                * Self::sigmoid_inv(activity_score, 28.0, 0.10)
                * Self::sigmoid_inv(delta_hr, 5.0, 0.15)
                * Self::sigmoid_inv(ppi_rmssd, 22.0, 0.15);
            candidates.push(EmotionCandidate { emotion: EmotionState::Exhausted, score: s, weight: 0.88 });
        }

        // RECOVERING
        {
            let trend_bonus = if hrv_trend > 0.5 { 1.0 } else { 0.2 };
            let s = trend_bonus
                * Self::bell(stress, 30.0, 20.0)
                * Self::sigmoid(sleep_score, 42.0, 0.06)
                * Self::sigmoid_inv(delta_hr, 12.0, 0.10)
                * Self::sigmoid(ppi_coherence, 0.32, 5.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Recovering, score: s, weight: 0.75 });
        }

        // FOCUSED
        {
            let s = Self::bell(hrv, 52.0, 22.0)
                * Self::bell(stress, 32.0, 15.0)
                * Self::bell(delta_hr, 10.0, 8.0)
                * Self::sigmoid(ppi_coherence, 0.42, 6.0)
                * Self::sigmoid_inv(activity_score, 52.0, 0.06)
                * Self::sigmoid(spo2, 95.5, 1.5);
            candidates.push(EmotionCandidate { emotion: EmotionState::Focused, score: s, weight: 0.78 });
        }

        // JOYFUL
        {
            let s = Self::sigmoid(hrv, 52.0, 0.08)
                * Self::sigmoid_inv(stress, 32.0, 0.10)
                * Self::bell(delta_hr, 12.0, 10.0)
                * Self::sigmoid(ppi_coherence, 0.52, 6.0)
                * Self::sigmoid(spo2, 96.5, 1.5)
                * Self::sigmoid(sleep_score, 52.0, 0.05)
                * Self::sigmoid(activity_score, 38.0, 0.05)
                * Self::sigmoid(temp_delta, -0.1, 3.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Joyful, score: s, weight: 0.72 });
        }

        // ENERGIZED
        {
            let s = Self::sigmoid(hrv, 48.0, 0.08)
                * Self::sigmoid_inv(stress, 38.0, 0.08)
                * Self::sigmoid(delta_hr, 8.0, 0.10)
                * Self::sigmoid(activity_score, 65.0, 0.06)
                * Self::sigmoid(spo2, 95.5, 1.5)
                * Self::sigmoid(sleep_score, 48.0, 0.04)
                * Self::sigmoid(ppi_coherence, 0.38, 5.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Energized, score: s, weight: 0.80 });
        }

        // RELAXED
        {
            let s = Self::sigmoid(hrv, 58.0, 0.08)
                * Self::sigmoid_inv(stress, 27.0, 0.10)
                * Self::sigmoid_inv(delta_hr, 9.0, 0.12)
                * Self::sigmoid(sleep_score, 58.0, 0.05)
                * Self::sigmoid(ppi_coherence, 0.48, 6.0)
                * Self::sigmoid(spo2, 96.5, 1.5)
                * Self::sigmoid_inv(activity_score, 52.0, 0.05);
            candidates.push(EmotionCandidate { emotion: EmotionState::Relaxed, score: s, weight: 0.85 });
        }

        // FEARFUL
        {
            let s = Self::sigmoid_inv(hrv, 28.0, 0.12)
                * Self::sigmoid_inv(spo2, 96.0, 2.0)
                * Self::sigmoid(stress, 60.0, 0.10)
                * Self::sigmoid_inv(ppi_coherence, 0.20, 10.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Fearful, score: s, weight: 0.90 });
        }

        // FRUSTRATED
        {
            let s = Self::sigmoid(stress, 45.0, 0.08)
                * Self::sigmoid_inv(hrv, 48.0, 0.08)
                * Self::bell(systolic_bp, 125.0, 15.0)
                * Self::bell(delta_hr, 10.0, 12.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Frustrated, score: s, weight: 0.76 });
        }

        // MEDITATIVE
        {
            let s = Self::sigmoid(hrv, 65.0, 0.10)
                * Self::sigmoid_inv(stress, 12.0, 0.15)
                * Self::sigmoid_inv(delta_hr, 3.0, 0.20)
                * Self::sigmoid(ppi_coherence, 0.65, 8.0)
                * Self::sigmoid_inv(activity_score, 15.0, 0.12)
                * Self::sigmoid(spo2, 97.0, 1.5);
            candidates.push(EmotionCandidate { emotion: EmotionState::Meditative, score: s, weight: 0.88 });
        }

        // DROWSY
        {
            let s = Self::sigmoid_inv(delta_hr, 2.0, 0.15)
                * Self::sigmoid_inv(hrv, 45.0, 0.06)
                * Self::sigmoid_inv(temp_delta, -0.1, 4.0)
                * Self::sigmoid_inv(activity_score, 10.0, 0.15)
                * Self::sigmoid_inv(stress, 25.0, 0.08);
            candidates.push(EmotionCandidate { emotion: EmotionState::Drowsy, score: s, weight: 0.74 });
        }

        // EXCITED
        {
            let s = Self::sigmoid(hrv, 55.0, 0.10)
                * Self::sigmoid_inv(stress, 25.0, 0.10)
                * Self::sigmoid(delta_hr, 18.0, 0.10)
                * Self::sigmoid(ppi_coherence, 0.50, 6.0)
                * Self::sigmoid(spo2, 96.5, 1.5)
                * Self::sigmoid(activity_score, 50.0, 0.05)
                * Self::sigmoid(temp_delta, 0.15, 4.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Excited, score: s, weight: 0.73 });
        }

        // PAIN
        {
            let s = Self::sigmoid(delta_hr, 10.0, 0.10)
                * Self::sigmoid(stress, 45.0, 0.08)
                * Self::sigmoid_inv(hrv, 40.0, 0.08)
                * Self::sigmoid(temp_delta, 0.3, 4.0)
                * Self::sigmoid_inv(activity_score, 20.0, 0.10)
                * Self::sigmoid_inv(ppi_coherence, 0.35, 6.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Pain, score: s, weight: 0.82 });
        }

        // FLOW
        {
            let s = Self::bell(hrv, 55.0, 15.0)
                * Self::bell(stress, 32.0, 10.0)
                * Self::bell(delta_hr, 8.0, 6.0)
                * Self::sigmoid(ppi_coherence, 0.55, 7.0)
                * Self::sigmoid(spo2, 96.5, 1.5);
            candidates.push(EmotionCandidate { emotion: EmotionState::Flow, score: s, weight: 0.85 });
        }

        // CALM (default positive)
        {
            let s = Self::sigmoid(hrv, 48.0, 0.06)
                * Self::sigmoid_inv(stress, 32.0, 0.08)
                * Self::sigmoid_inv(delta_hr.abs(), 12.0, 0.10)
                * Self::sigmoid(spo2, 95.5, 1.0)
                * Self::sigmoid(ppi_coherence, 0.38, 4.0);
            candidates.push(EmotionCandidate { emotion: EmotionState::Calm, score: s, weight: 0.70 });
        }

        // Sort by weighted score
        candidates.sort_by(|a, b| {
            let sa = b.score * b.weight;
            let sb = a.score * a.weight;
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Temporal smoothing: don't change emotion within 5 min unless 30% stronger
        let mut top = candidates[0].clone();
        if elapsed_secs < 300.0 {
            if let Some(prev) = prev_emotion {
                if top.emotion != prev {
                    if let Some(prev_cand) = candidates.iter().find(|c| c.emotion == prev) {
                        let top_w = top.score * top.weight;
                        let prev_w = prev_cand.score * prev_cand.weight;
                        if top_w < prev_w * 1.3 {
                            top = prev_cand.clone();
                        }
                    }
                }
            }
        }

        let secondary = if candidates.len() > 1 && candidates[1].emotion != top.emotion {
            candidates[1].clone()
        } else if candidates.len() > 2 {
            candidates[2].clone()
        } else {
            top.clone()
        };

        EmotionResult {
            primary: top.emotion,
            primary_confidence: (top.score * top.weight).min(1.0),
            secondary: secondary.emotion,
            secondary_confidence: (secondary.score * secondary.weight).min(1.0),
            emoji: top.emotion.emoji().to_string(),
            category: top.emotion.category().to_string(),
            label: top.emotion.label().to_string(),
            all_scores: candidates,
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_args() -> (f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64) {
        // heart_rate, resting_hr, hrv, stress, spo2, temperature, base_temp,
        // systolic_bp, ppi_coherence, ppi_rmssd, sleep_score, activity_score, hrv_trend
        (72.0, 65.0, 55.0, 35.0, 98.0, 36.6, 36.6, 118.0, 0.65, 45.0, 78.0, 50.0, 0.0)
    }

    #[test]
    fn detect_always_returns_primary_emotion() {
        let (hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr) = baseline_args();
        let result = EmotionEngine::detect(
            hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr, None, 0.0,
        );
        // Must pick some primary emotion and assign non-negative confidence
        assert!(result.primary_confidence >= 0.0);
        assert!(result.primary_confidence <= 1.0);
        assert_eq!(result.all_scores.len(), 18, "All 18 candidates must be in all_scores");
    }

    #[test]
    fn detect_high_stress_leans_negative() {
        // High stress, high HR, low HRV → should lean angry/anxious/stressed
        let result = EmotionEngine::detect(
            105.0, 65.0, 18.0, 85.0, 97.0, 36.6, 36.6, 135.0, 0.25, 20.0, 40.0, 60.0, -1.0, None, 0.0,
        );
        let negative = [
            EmotionState::Angry, EmotionState::Anxious, EmotionState::Stressed,
            EmotionState::Frustrated, EmotionState::Fearful,
        ];
        assert!(
            negative.contains(&result.primary),
            "High stress should pick a negative emotion, got {:?}", result.primary
        );
    }

    #[test]
    fn detect_high_hrv_low_stress_leans_positive() {
        // Recovery-state: high HRV, low stress, normal HR, good sleep
        let result = EmotionEngine::detect(
            65.0, 62.0, 88.0, 20.0, 98.0, 36.6, 36.6, 115.0, 0.75, 60.0, 85.0, 30.0, 1.0, None, 0.0,
        );
        let positive = [
            EmotionState::Calm, EmotionState::Relaxed, EmotionState::Recovering,
            EmotionState::Meditative, EmotionState::Focused,
        ];
        assert!(
            positive.contains(&result.primary),
            "Recovery state should pick a positive emotion, got {:?}", result.primary
        );
    }

    #[test]
    fn detect_returns_timestamp() {
        let (hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr) = baseline_args();
        let result = EmotionEngine::detect(
            hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr, None, 0.0,
        );
        // Timestamp must be recent (< 1 minute old)
        let age = (Utc::now() - result.timestamp).num_seconds();
        assert!(age >= 0 && age < 60);
    }

    #[test]
    fn detect_returns_emoji_and_label() {
        let (hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr) = baseline_args();
        let result = EmotionEngine::detect(
            hr, rhr, hrv, stress, spo2, temp, btemp, bp, coh, pp, sl, act, tr, None, 0.0,
        );
        assert!(!result.emoji.is_empty());
        assert!(!result.label.is_empty());
        assert!(!result.category.is_empty());
    }

    #[test]
    fn detect_is_deterministic_for_same_inputs() {
        let args = baseline_args();
        let r1 = EmotionEngine::detect(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, args.8, args.9,
            args.10, args.11, args.12, None, 0.0,
        );
        let r2 = EmotionEngine::detect(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, args.8, args.9,
            args.10, args.11, args.12, None, 0.0,
        );
        // Same inputs must pick same primary emotion
        assert_eq!(r1.primary, r2.primary);
    }
}
