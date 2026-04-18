use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq, Hash)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum MetricType {
    Hr,
    Hrv,
    Spo2,
    Temp,
    Wvi,
    Stress,
    EmotionConfidence,
    Energy,
    Recovery,
    Coherence,
    BreathingRate,
    ActivityIntensity,
}

impl MetricType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hr => "hr",
            Self::Hrv => "hrv",
            Self::Spo2 => "spo2",
            Self::Temp => "temp",
            Self::Wvi => "wvi",
            Self::Stress => "stress",
            Self::EmotionConfidence => "emotion_confidence",
            Self::Energy => "energy",
            Self::Recovery => "recovery",
            Self::Coherence => "coherence",
            Self::BreathingRate => "breathing_rate",
            Self::ActivityIntensity => "activity_intensity",
        }
    }
    pub fn all() -> &'static [Self] {
        &[
            Self::Hr,
            Self::Hrv,
            Self::Spo2,
            Self::Temp,
            Self::Wvi,
            Self::Stress,
            Self::EmotionConfidence,
            Self::Energy,
            Self::Recovery,
            Self::Coherence,
            Self::BreathingRate,
            Self::ActivityIntensity,
        ]
    }
    pub fn is_derived(&self) -> bool {
        matches!(
            self,
            Self::Wvi | Self::Stress | Self::EmotionConfidence | Self::Energy | Self::Recovery | Self::Coherence
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChartPoint {
    pub ts: DateTime<Utc>,
    pub value: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartEvent {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub event_type: String,
    pub meta: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntradayResponse {
    pub metric: String,
    pub period: String,
    pub resolution: String,
    pub points: Vec<ChartPoint>,
    pub events: Vec<ChartEvent>,
    pub compare_points: Option<Vec<ChartPoint>>,
    pub formula_version: i32,
    pub backfill_in_progress: bool,
    pub backfill_progress: f64,
}
