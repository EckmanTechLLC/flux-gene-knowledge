/// Steering publisher — publishes knowledge-gene/steer to Flux.
///
/// Only fires when:
///   - `--steer` flag is set (caller's responsibility to gate)
///   - interpretation confidence ≥ 0.80
///   - suggested_actions is non-empty
///
/// Publishes to Flux /api/events as entity `knowledge-gene/steer`.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::context::Interpretation;
use crate::publisher::FluxPublisher;

/// Publish a steer command to Flux if confidence and actions warrant it.
pub async fn apply(publisher: &FluxPublisher, interp: &Interpretation) {
    if interp.confidence < 0.80 {
        tracing::debug!(
            "steer: skipped — confidence {:.2} < 0.80",
            interp.confidence
        );
        return;
    }
    if interp.suggested_actions.is_empty() {
        tracing::debug!("steer: skipped — no suggested actions");
        return;
    }

    if let Err(e) = publish_steer(publisher, interp).await {
        tracing::warn!("steer: publish failed: {}", e);
    }
}

async fn publish_steer(publisher: &FluxPublisher, interp: &Interpretation) -> Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let reason: String = interp.interpretation.chars().take(200).collect();

    let properties = serde_json::json!({
        "action_ids":  interp.suggested_actions,
        "confidence":  interp.confidence,
        "tick_ref":    interp.tick_ref,
        "reason":      reason,
    });

    let body = serde_json::json!({
        "stream":    "knowledge.gene",
        "source":    "knowledge-gene",
        "timestamp": ts,
        "payload": {
            "entity_id":  "knowledge-gene/steer",
            "properties": properties,
        }
    });

    publisher.post_raw(body).await?;

    tracing::info!(
        "steer: published action_ids={:?} confidence={:.2} tick_ref={}",
        interp.suggested_actions,
        interp.confidence,
        interp.tick_ref,
    );

    Ok(())
}
