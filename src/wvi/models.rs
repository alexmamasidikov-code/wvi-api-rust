use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WVILevel {
    Superb,
    Excellent,
    Good,
    Moderate,
    Low,
    Poor,
    Dangerous,
}

impl WVILevel {
    pub fn from_score(score: f64) -> Self {
        match score as u32 {
            90..=100 => Self::Superb,
            80..=89 => Self::Excellent,
            65..=79 => Self::Good,
            50..=64 => Self::Moderate,
            35..=49 => Self::Low,
            20..=34 => Self::Poor,
            _ => Self::Dangerous,
        }
    }
}

impl std::fmt::Display for WVILevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Superb => "superb",
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Moderate => "moderate",
            Self::Low => "low",
            Self::Poor => "poor",
            Self::Dangerous => "dangerous",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricScores {
    pub heart_rate: f64,
    pub hrv: f64,
    pub stress: f64,
    pub spo2: f64,
    pub temperature: f64,
    pub sleep: f64,
    pub activity: f64,
    pub blood_pressure: f64,
    pub ppi_coherence: f64,
    pub emotional_wellbeing: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricWeights {
    pub hrv: f64,
    pub stress: f64,
    pub sleep: f64,
    pub emotion: f64,
    pub spo2: f64,
    pub heart_rate: f64,
    pub activity: f64,
    pub blood_pressure: f64,
    pub temperature: f64,
    pub ppi: f64,
}

impl Default for MetricWeights {
    fn default() -> Self {
        Self {
            hrv: 0.18,
            stress: 0.15,
            sleep: 0.13,
            emotion: 0.12,
            spo2: 0.09,
            heart_rate: 0.09,
            activity: 0.08,
            blood_pressure: 0.06,
            temperature: 0.05,
            ppi: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WVISnapshot {
    pub wvi_score: f64,
    pub level: WVILevel,
    pub metrics: MetricScores,
    pub weights: MetricWeights,
    pub emotion_feedback: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMetrics {
    pub heart_rate: f64,
    pub resting_hr: f64,
    pub hrv: f64,
    pub stress: f64,
    pub spo2: f64,
    pub temperature: f64,
    pub base_temp: f64,
    pub systolic_bp: f64,
    pub diastolic_bp: f64,
    pub ppi_rmssd: f64,
    pub ppi_coherence: f64,
    pub total_sleep_minutes: f64,
    pub deep_sleep_percent: f64,
    pub sleep_continuity: f64,
    pub steps: f64,
    pub active_minutes: f64,
    pub mets: f64,
    pub age: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WVIHistoryQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub granularity: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateRequest {
    pub heart_rate: Option<f64>,
    pub resting_hr: Option<f64>,
    pub hrv: Option<f64>,
    pub stress: Option<f64>,
    pub spo2: Option<f64>,
    pub temperature: Option<f64>,
    pub sleep_hours: Option<f64>,
    pub sleep_score: Option<f64>,
    pub steps: Option<f64>,
    pub active_calories: Option<f64>,
    pub acwr: Option<f64>,
    pub systolic_bp: Option<f64>,
    pub diastolic_bp: Option<f64>,
    pub ppi_coherence: Option<f64>,
    pub emotion_name: Option<String>,
    pub emotion_score: Option<f64>,
}
