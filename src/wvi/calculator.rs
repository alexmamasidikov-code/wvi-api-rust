use super::models::*;
use super::normalizer::MetricNormalizer;
use crate::emotions::models::EmotionState;
use chrono::Utc;

pub struct WVICalculator;

impl WVICalculator {
    /// Calculate WVI score from raw metrics with adaptive weights
    pub fn calculate(raw: &RawMetrics, hour: u32, is_exercising: bool, emotion: Option<EmotionState>, emotion_confidence: f64) -> WVISnapshot {
        let scores = MetricNormalizer::normalize_all(raw);
        let weights = Self::adaptive_weights(hour, is_exercising);

        let total_weight = weights.hrv + weights.stress + weights.sleep + weights.spo2
            + weights.heart_rate + weights.activity + weights.blood_pressure
            + weights.temperature + weights.ppi;

        let raw_wvi = (scores.hrv * weights.hrv
            + scores.stress * weights.stress
            + scores.sleep * weights.sleep
            + scores.emotional_wellbeing * weights.emotion
            + scores.spo2 * weights.spo2
            + scores.heart_rate * weights.heart_rate
            + scores.activity * weights.activity
            + scores.blood_pressure * weights.blood_pressure
            + scores.temperature * weights.temperature
            + scores.ppi_coherence * weights.ppi)
            / total_weight;

        let emotion_multiplier = emotion
            .map(|e| Self::emotion_feedback(e, emotion_confidence))
            .unwrap_or(1.0);

        let final_wvi = (raw_wvi * emotion_multiplier).clamp(0.0, 100.0);

        WVISnapshot {
            wvi_score: (final_wvi * 10.0).round() / 10.0,
            level: WVILevel::from_score(final_wvi),
            metrics: scores,
            weights,
            emotion_feedback: emotion_multiplier,
            timestamp: Utc::now(),
        }
    }

    /// Adaptive weights based on time of day and exercise state
    fn adaptive_weights(hour: u32, is_exercising: bool) -> MetricWeights {
        let mut w = MetricWeights::default();

        match hour {
            22..=23 | 0..=5 => {
                w.sleep = 0.25; w.temperature = 0.08; w.activity = 0.03;
                w.hrv = 0.20; w.stress = 0.16;
            }
            6..=9 => {
                w.hrv = 0.28; w.sleep = 0.18; w.stress = 0.15;
                w.activity = 0.05;
            }
            10..=17 => {
                w.stress = 0.22; w.hrv = 0.20; w.activity = 0.12;
            }
            _ => {}
        }

        if is_exercising {
            w.heart_rate = 0.05;
            w.activity = 0.15;
            w.spo2 = 0.15;
        }

        w
    }

    /// Emotion feedback loop: positive emotions boost WVI, negative reduce
    fn emotion_feedback(emotion: EmotionState, confidence: f64) -> f64 {
        let multiplier = match emotion {
            EmotionState::Flow => 1.12,
            EmotionState::Meditative => 1.10,
            EmotionState::Joyful => 1.08,
            EmotionState::Excited => 1.06,
            EmotionState::Energized => 1.05,
            EmotionState::Relaxed => 1.04,
            EmotionState::Calm => 1.02,
            EmotionState::Focused => 1.03,
            EmotionState::Recovering => 1.00,
            EmotionState::Drowsy => 0.97,
            EmotionState::Stressed => 0.95,
            EmotionState::Frustrated => 0.93,
            EmotionState::Sad => 0.91,
            EmotionState::Anxious => 0.88,
            EmotionState::Angry => 0.87,
            EmotionState::Pain => 0.86,
            EmotionState::Fearful => 0.85,
            EmotionState::Exhausted => 0.85,
        };

        1.0 + (multiplier - 1.0) * confidence
    }
}
