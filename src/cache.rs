use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Instant, Duration};

#[derive(Clone)]
pub struct AppCache {
    user_ids: Arc<RwLock<HashMap<String, (uuid::Uuid, Instant)>>>,
    ai_responses: Arc<RwLock<HashMap<String, (String, Instant)>>>,
}

impl AppCache {
    pub fn new() -> Self {
        Self {
            user_ids: Arc::new(RwLock::new(HashMap::new())),
            ai_responses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_user_id(&self, privy_did: &str) -> Option<uuid::Uuid> {
        let cache = self.user_ids.read().await;
        cache.get(privy_did).and_then(|(id, ts)| {
            if ts.elapsed() < Duration::from_secs(300) { Some(*id) } else { None }
        })
    }

    pub async fn set_user_id(&self, privy_did: &str, id: uuid::Uuid) {
        let mut cache = self.user_ids.write().await;
        cache.insert(privy_did.to_string(), (id, Instant::now()));
    }

    pub async fn get_ai(&self, key: &str) -> Option<String> {
        let cache = self.ai_responses.read().await;
        cache.get(key).and_then(|(resp, ts)| {
            if ts.elapsed() < Duration::from_secs(600) { Some(resp.clone()) } else { None }
        })
    }

    pub async fn set_ai(&self, key: &str, response: String) {
        let mut cache = self.ai_responses.write().await;
        cache.insert(key.to_string(), (response, Instant::now()));
    }
}
