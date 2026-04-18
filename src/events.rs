use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

type Buffers = Arc<Mutex<HashMap<String, Vec<(String, String)>>>>;

#[derive(Clone)]
pub struct EventBus {
    producer: FutureProducer,
    buffers: Buffers,
}

#[derive(Debug, Serialize)]
pub struct BiometricEvent {
    pub user_id: String,
    pub event_type: String,
    pub timestamp: String,
    pub data: serde_json::Value,
}

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
            .set("linger.ms", "100")
            .set("batch.num.messages", "10000")
            .set("compression.type", "lz4")
            .create()?;
        let bus = Self { producer, buffers: Arc::new(Mutex::new(HashMap::new())) };
        bus.spawn_flusher();
        bus.spawn_shutdown_hook();
        Ok(bus)
    }

    fn flush_interval() -> Duration {
        let ms = std::env::var("KAFKA_BATCH_MS").ok()
            .and_then(|v| v.parse().ok()).unwrap_or(100);
        Duration::from_millis(ms)
    }

    fn spawn_flusher(&self) {
        let producer = self.producer.clone();
        let buffers = self.buffers.clone();
        let interval = Self::flush_interval();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                Self::flush_all(&producer, &buffers).await;
            }
        });
    }

    fn spawn_shutdown_hook(&self) {
        let producer = self.producer.clone();
        let buffers = self.buffers.clone();
        tokio::spawn(async move {
            let mut term = match tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate()) { Ok(s) => s, Err(_) => return };
            term.recv().await;
            tracing::info!("SIGTERM — draining Kafka event buffer");
            Self::flush_all(&producer, &buffers).await;
        });
    }

    async fn flush_all(producer: &FutureProducer, buffers: &Buffers) {
        let drained: Vec<(String, Vec<(String, String)>)> = {
            let mut g = buffers.lock().await;
            g.drain().filter(|(_, v)| !v.is_empty()).collect()
        };
        for (topic, msgs) in drained {
            let n = msgs.len();
            for (key, payload) in &msgs {
                let record = FutureRecord::to(&topic).key(key).payload(payload);
                if let Err((e, _)) = producer.send_result(record) {
                    tracing::warn!("Kafka enqueue failed for {topic}: {e}");
                }
            }
            tracing::debug!("Flushed {n} events to {topic}");
        }
    }

    pub async fn publish(&self, topic: &str, key: &str, event: &impl Serialize) {
        let payload = match serde_json::to_string(event) {
            Ok(p) => p,
            Err(e) => { tracing::warn!("Failed to serialize event: {e}"); return; }
        };
        let mut g = self.buffers.lock().await;
        g.entry(topic.to_string()).or_default().push((key.to_string(), payload));
    }

    pub fn noop() -> Self {
        Self {
            producer: ClientConfig::new().set("bootstrap.servers", "localhost:9092")
                .create().expect("Failed to create noop producer"),
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
