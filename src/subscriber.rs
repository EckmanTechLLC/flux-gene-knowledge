/// Flux WebSocket subscriber for observer-gene entities.
///
/// Maintains a shared state map:  entity_id → property → JSON value
///
/// Threading pattern: identical to observer-gene/signal/flux_multi.rs —
/// do not deviate.  The async task holds the std::sync::Mutex lock only
/// in the synchronous handle_message function; it is always released before
/// any .await point, so using std::sync::Mutex in tokio code is safe here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

// The three entities we subscribe to.
const WATCHED: &[&str] = &[
    "observer-gene/state",
    "observer-gene/symbols",
    "observer-gene/key",
];

/// Shared state between the WS background task and the main loop.
pub struct ObserverState {
    /// entity_id → property_name → most recent JSON value
    pub entities:  HashMap<String, HashMap<String, serde_json::Value>>,
    pub connected: bool,
}

impl ObserverState {
    pub fn new() -> Self {
        Self { entities: HashMap::new(), connected: false }
    }
}

/// Spawn the WS subscriber as a tokio task.  Returns the shared state handle.
pub fn spawn(ws_url: String) -> Arc<Mutex<ObserverState>> {
    let state = Arc::new(Mutex::new(ObserverState::new()));
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move { run(ws_url, state_clone).await });
    state
}

// ── Background task ───────────────────────────────────────────────────────────

async fn run(url: String, state: Arc<Mutex<ObserverState>>) {
    loop {
        tracing::info!("subscriber: connecting to {}", url);
        match connect_and_listen(url.clone(), Arc::clone(&state)).await {
            Ok(())  => tracing::info!("subscriber: connection closed cleanly"),
            Err(e)  => tracing::warn!("subscriber: error: {}", e),
        }
        if let Ok(mut s) = state.lock() {
            s.connected = false;
        }
        tracing::info!("subscriber: reconnecting in 10s");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn connect_and_listen(
    url:   String,
    state: Arc<Mutex<ObserverState>>,
) -> Result<()> {
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;

    // Subscribe to all entities; we filter client-side by entity_id.
    ws.send(Message::Text(
        r#"{"type":"subscribe","entity_id":"*"}"#.to_string().into(),
    )).await?;

    if let Ok(mut s) = state.lock() {
        s.connected = true;
    }
    tracing::info!("subscriber: subscribed to all entities");

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    handle_message(&json, &state);
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

fn handle_message(json: &serde_json::Value, state: &Arc<Mutex<ObserverState>>) {
    let msg_type = match json.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None    => return,
    };

    match msg_type {
        "state_update" => {
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) if WATCHED.contains(&id) => id.to_string(),
                _ => return,
            };
            let property = match json.get("property").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None    => return,
            };
            let value = match json.get("value") {
                Some(v) => v.clone(),
                None    => return,
            };
            if let Ok(mut s) = state.lock() {
                s.entities
                    .entry(entity_id)
                    .or_default()
                    .insert(property, value);
            }
        }
        "entity_deleted" => {
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) if WATCHED.contains(&id) => id,
                _ => return,
            };
            if let Ok(mut s) = state.lock() {
                s.entities.remove(entity_id);
            }
        }
        _ => {} // metrics_update and unknown types silently ignored
    }
}
