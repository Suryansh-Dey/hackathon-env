//! AI output cache keyed by agent, prompt version and input hash (PLAN.md §11.12).

use crate::model::Part;
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Storage for validated agent outputs. The in-memory implementation serves a
/// warm Lambda; the main tree backs it with the DynamoDB `ephemeral` table.
#[async_trait]
pub trait OutputCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<String>;
    async fn put(&self, key: &str, value: String);
}

/// Bounded FIFO cache.
pub struct InMemoryCache {
    capacity: usize,
    inner: Mutex<(HashMap<String, String>, VecDeque<String>)>,
}

impl InMemoryCache {
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), inner: Mutex::new((HashMap::new(), VecDeque::new())) }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new(512)
    }
}

#[async_trait]
impl OutputCache for InMemoryCache {
    async fn get(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap().0.get(key).cloned()
    }

    async fn put(&self, key: &str, value: String) {
        let mut g = self.inner.lock().unwrap();
        let (map, order) = &mut *g;
        if map.insert(key.to_string(), value).is_none() {
            order.push_back(key.to_string());
        }
        while map.len() > self.capacity {
            if let Some(old) = order.pop_front() {
                map.remove(&old);
            }
        }
    }
}

/// Cache that never stores anything.
pub struct NoCache;

#[async_trait]
impl OutputCache for NoCache {
    async fn get(&self, _key: &str) -> Option<String> {
        None
    }
    async fn put(&self, _key: &str, _value: String) {}
}

/// `sha256(agent, prompt version, model, system prompt, schema, inputs)`, with images hashed by content.
pub fn cache_key(agent: &str, prompt_version: &str, model: &str, system: &str, schema: &Value, parts: &[Part]) -> String {
    let parts: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => json!({"t": t}),
            Part::Image { mime, bytes, .. } => json!({"i": hex::encode(Sha256::digest(bytes)), "m": mime}),
        })
        .collect();
    let material = json!({"a": agent, "v": prompt_version, "m": model, "s": system, "j": schema, "p": parts});
    format!("{agent}:{prompt_version}:{}", hex::encode(Sha256::digest(material.to_string().as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_and_keyed() {
        let c = InMemoryCache::new(2);
        c.put("a", "1".into()).await;
        c.put("b", "2".into()).await;
        c.put("c", "3".into()).await;
        assert_eq!(c.len(), 2);
        assert!(c.get("a").await.is_none());
        assert_eq!(c.get("c").await.as_deref(), Some("3"));
    }

    #[test]
    fn key_depends_on_image_content_not_id() {
        let img = |id: &str, b: u8| Part::Image { image_id: id.into(), mime: "image/png".into(), bytes: vec![b; 8] };
        let k1 = cache_key("triage", "v1", "m", "sys", &json!({}), &[img("img_a", 1)]);
        let k2 = cache_key("triage", "v1", "m", "sys", &json!({}), &[img("img_b", 1)]);
        let k3 = cache_key("triage", "v1", "m", "sys", &json!({}), &[img("img_a", 2)]);
        let k4 = cache_key("triage", "v2", "m", "sys", &json!({}), &[img("img_a", 1)]);
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
        assert!(k1.starts_with("triage:v1:"));
    }
}
