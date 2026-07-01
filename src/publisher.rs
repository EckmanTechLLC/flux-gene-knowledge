/// Flux HTTP publisher for knowledge-gene/state.
///
/// Publishes LLM interpretation results back to Flux Universe using the same
/// HTTP POST /api/events format as observer-gene's publisher.rs.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::context::Interpretation;
use crate::lessons::Lesson;

pub struct FluxPublisher {
    client:    reqwest::Client,
    http_base: String,
    token:     Option<String>,
}

impl FluxPublisher {
    pub fn new(http_base: String, token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            http_base,
            token,
        }
    }

    /// POST a raw JSON body to Flux /api/events.
    /// Used by steer.rs to publish knowledge-gene/steer events.
    pub async fn post_raw(&self, body: serde_json::Value) -> Result<()> {
        let url = format!("{}/api/events", self.http_base.trim_end_matches('/'));
        let mut req = self.client.post(&url).json(&body);
        if let Some(ref tok) = self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            anyhow::bail!("flux HTTP {}: {}", status, &text[..text.len().min(200)]);
        }
        Ok(())
    }

    /// POST an interpretation to Flux as knowledge-gene/state.
    pub async fn publish(
        &self,
        interp: &Interpretation,
        key:    Option<&HashMap<String, serde_json::Value>>,
    ) -> Result<()> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let action_map = key
            .and_then(|k| k.get("actions"))
            .and_then(|v| v.as_object());

        let suggested_action_labels: Vec<String> = interp.suggested_actions
            .iter()
            .map(|&n| {
                action_map
                    .and_then(|m| m.get(&format!("action_{}", n)))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("action_{}", n))
            })
            .collect();

        let properties = serde_json::json!({
            "interpretation":         interp.interpretation,
            "suggested_actions":      interp.suggested_actions,
            "suggested_action_labels": suggested_action_labels,
            "confidence":             interp.confidence,
            "tick_ref":               interp.tick_ref,
            "themes":                 interp.themes,
        });

        let body = serde_json::json!({
            "stream":    "knowledge.gene",
            "source":    "knowledge-gene",
            "timestamp": ts,
            "payload": {
                "entity_id":  "knowledge-gene/state",
                "properties": properties,
            }
        });

        let url = format!("{}/api/events", self.http_base.trim_end_matches('/'));
        let mut req = self.client.post(&url).json(&body);
        if let Some(ref tok) = self.token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            tracing::warn!("flux publish failed {}: {}", status, &text[..text.len().min(200)]);
        } else {
            tracing::info!(
                "published knowledge-gene/state (tick_ref={}, confidence={:.2})",
                interp.tick_ref,
                interp.confidence,
            );
        }
        Ok(())
    }

    /// Publish a single Lesson to the knowledge-gene/lessons Flux entity.
    ///
    /// The property key is `lesson_NNNNNNNNNNNNN` (tick_range_end zero-padded
    /// to 13 digits), giving lexicographic = numeric ordering.
    pub async fn publish_lesson(&self, lesson: &Lesson) -> Result<()> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let key = format!("lesson_{:013}", lesson.tick_range_end);

        let mut properties = serde_json::Map::new();
        properties.insert(key.clone(), serde_json::to_value(lesson)?);
        let properties = serde_json::Value::Object(properties);

        let body = serde_json::json!({
            "stream":    "knowledge.gene",
            "source":    "knowledge-gene",
            "timestamp": ts,
            "payload": {
                "entity_id":  "knowledge-gene/lessons",
                "properties": properties,
            }
        });

        self.post_raw(body).await?;
        tracing::info!(
            "published lesson tick_range={}..{} to knowledge-gene/lessons (key={})",
            lesson.tick_range_start,
            lesson.tick_range_end,
            key,
        );
        Ok(())
    }

    /// Delete a lesson from the knowledge-gene/lessons Flux entity by writing
    /// a null value to its property key.
    pub async fn delete_lesson(&self, end_tick: u64) -> Result<()> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let key = format!("lesson_{:013}", end_tick);

        let mut properties = serde_json::Map::new();
        properties.insert(key.clone(), serde_json::Value::Null);
        let properties = serde_json::Value::Object(properties);

        let body = serde_json::json!({
            "stream":    "knowledge.gene",
            "source":    "knowledge-gene",
            "timestamp": ts,
            "payload": {
                "entity_id":  "knowledge-gene/lessons",
                "properties": properties,
            }
        });

        self.post_raw(body).await?;
        tracing::info!("deleted lesson key={} from knowledge-gene/lessons", key);
        Ok(())
    }
}
