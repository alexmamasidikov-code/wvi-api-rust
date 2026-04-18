use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeekdayType { Weekday, Weekend }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeOfDay { Morning, Afternoon, Evening, Night }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivityState { Resting, Active, PostWorkout1h }

#[derive(Debug, Clone, PartialEq)]
pub struct ContextKey {
    pub weekday_type: WeekdayType,
    pub time_of_day: TimeOfDay,
    pub activity_state: ActivityState,
}

impl ContextKey {
    pub fn from_ts(ts: DateTime<Utc>, activity: ActivityState) -> Self {
        use chrono::{Datelike, Timelike};
        let weekday = ts.weekday().num_days_from_monday();
        let wtype = if weekday >= 5 { WeekdayType::Weekend } else { WeekdayType::Weekday };
        let hour = ts.hour();
        let tod = match hour {
            6..=11 => TimeOfDay::Morning,
            12..=17 => TimeOfDay::Afternoon,
            18..=21 => TimeOfDay::Evening,
            _ => TimeOfDay::Night,
        };
        Self { weekday_type: wtype, time_of_day: tod, activity_state: activity }
    }

    pub fn as_str(&self) -> String {
        let w = match self.weekday_type {
            WeekdayType::Weekday => "weekday",
            WeekdayType::Weekend => "weekend",
        };
        let t = match self.time_of_day {
            TimeOfDay::Morning => "morning",
            TimeOfDay::Afternoon => "afternoon",
            TimeOfDay::Evening => "evening",
            TimeOfDay::Night => "night",
        };
        let a = match self.activity_state {
            ActivityState::Resting => "resting",
            ActivityState::Active => "active",
            ActivityState::PostWorkout1h => "post_workout_1h",
        };
        format!("{}_{}_{}", w, t, a)
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Baseline {
    pub mean: f64,
    pub std: f64,
    pub p10: f64,
    pub p90: f64,
    pub sample_count: i32,
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Up, Down }

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Signal {
    pub id: Uuid,
    pub user_id: Uuid,
    pub ts: DateTime<Utc>,
    pub metric_type: String,
    pub context_key: String,
    pub deviation_sigma: f64,
    pub direction: String,
    pub severity: String,
    pub detectors_fired: serde_json::Value,
    pub bayesian_confidence: Option<f64>,
    pub rarity_percentile: Option<f64>,
    pub narrative: Option<String>,
    pub ack: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CusumState { pub s_plus: f64, pub s_minus: f64 }
impl Default for CusumState { fn default() -> Self { Self { s_plus: 0.0, s_minus: 0.0 } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EwmaState { pub z: f64 }
impl Default for EwmaState { fn default() -> Self { Self { z: 0.0 } } }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetectorState {
    pub cusum: CusumState,
    pub ewma: EwmaState,
}
