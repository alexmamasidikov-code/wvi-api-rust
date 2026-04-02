use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EmotionState {
    // Positive (5)
    Calm,
    Relaxed,
    Joyful,
    Energized,
    Excited,
    // Neutral/Productive (4)
    Focused,
    Meditative,
    Recovering,
    Drowsy,
    // Negative (7)
    Stressed,
    Anxious,
    Angry,
    Frustrated,
    Fearful,
    Sad,
    Exhausted,
    // Physiological (2)
    Pain,
    Flow,
}

impl EmotionState {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Calm | Self::Relaxed | Self::Joyful | Self::Energized | Self::Excited => "positive",
            Self::Focused | Self::Meditative | Self::Recovering | Self::Drowsy => "neutral",
            Self::Stressed | Self::Anxious | Self::Angry | Self::Frustrated
            | Self::Fearful | Self::Sad | Self::Exhausted => "negative",
            Self::Pain | Self::Flow => "physiological",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Calm => "😌", Self::Relaxed => "🧘", Self::Joyful => "😊",
            Self::Energized => "⚡", Self::Excited => "🎉",
            Self::Focused => "🎯", Self::Meditative => "🕉", Self::Recovering => "🔄",
            Self::Drowsy => "😴",
            Self::Stressed => "😰", Self::Anxious => "😱", Self::Angry => "😤",
            Self::Frustrated => "😣", Self::Fearful => "😨", Self::Sad => "😔",
            Self::Exhausted => "😩",
            Self::Pain => "🤕", Self::Flow => "🌊",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Calm => "Спокойствие", Self::Relaxed => "Расслабленность",
            Self::Joyful => "Радость", Self::Energized => "Энергичность",
            Self::Excited => "Возбуждение",
            Self::Focused => "Концентрация", Self::Meditative => "Медитация",
            Self::Recovering => "Восстановление", Self::Drowsy => "Сонливость",
            Self::Stressed => "Стресс", Self::Anxious => "Тревожность",
            Self::Angry => "Гнев", Self::Frustrated => "Фрустрация",
            Self::Fearful => "Страх", Self::Sad => "Подавленность",
            Self::Exhausted => "Истощение",
            Self::Pain => "Дискомфорт", Self::Flow => "Состояние потока",
        }
    }

    pub fn all() -> &'static [EmotionState] {
        &[
            Self::Calm, Self::Relaxed, Self::Joyful, Self::Energized, Self::Excited,
            Self::Focused, Self::Meditative, Self::Recovering, Self::Drowsy,
            Self::Stressed, Self::Anxious, Self::Angry, Self::Frustrated,
            Self::Fearful, Self::Sad, Self::Exhausted,
            Self::Pain, Self::Flow,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmotionCandidate {
    pub emotion: EmotionState,
    pub score: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmotionResult {
    pub primary: EmotionState,
    pub primary_confidence: f64,
    pub secondary: EmotionState,
    pub secondary_confidence: f64,
    pub emoji: String,
    pub category: String,
    pub label: String,
    pub all_scores: Vec<EmotionCandidate>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
