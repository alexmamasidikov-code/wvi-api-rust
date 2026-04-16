use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone)]
pub struct EventBus {
    producer: FutureProducer,
}

#[derive(Debug, Serialize)]
pub struct BiometricEvent {
    pub user_id: String,
    pub event_type: String,
    pub timestamp: String,
    pub data: serde_json::Value,
}

// Topics
pub const TOPIC_BIOMETRICS: &str = "wvi.biometrics";
pub const TOPIC_EMOTIONS: &str = "wvi.emotions";
pub const TOPIC_WVI: &str = "wvi.scores";
pub const TOPIC_ALERTS: &str = "wvi.alerts";
pub const TOPIC_AUDIT: &str = "wvi.audit";

impl EventBus {
    pub fn new(brokers: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .set("queue.buffering.max.ms", "100")
            .create()?;
        Ok(Self { producer })
    }

    pub async fn publish(&self, topic: &str, key: &str, event: &impl Serialize) {
        let payload = match serde_json::to_string(event) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to serialize event: {e}");
                return;
            }
        };

        let record = FutureRecord::to(topic)
            .key(key)
            .payload(&payload);

        match self.producer.send(record, Duration::from_secs(5)).await {
            Ok(_) => tracing::debug!("Event published to {topic}"),
            Err((e, _)) => tracing::warn!("Failed to publish to {topic}: {e}"),
        }
    }

    /// Create a no-op event bus when Kafka is not configured
    pub fn noop() -> Self {
        // Will fail on publish but won't crash
        Self {
            producer: ClientConfig::new()
                .set("bootstrap.servers", "localhost:9092")
                .create()
                .expect("Failed to create noop producer"),
        }
    }
}
