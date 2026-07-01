/// knowledge-gene — LLM interpreter companion for observer-gene.
///
/// Subscribes to observer-gene's Flux entities, builds a rolling context
/// window of state history and prior interpretations, and periodically
/// sends it to an OpenAI-compatible LLM.  Publishes the interpretation
/// back to Flux as knowledge-gene/state.
///
/// ## Run modes
///   Observe-only (default): reads observer-gene state, interprets, publishes.
///   Steering (--steer):     additionally pushes suggested_actions to observer-gene.
///                           Not wired yet — steer::apply() is a stub.
///
/// ## Rate limiting
///   LLM calls fire on a configurable interval (default 300s / 5 min).
///   Significant state changes (dominant shift, new symbol, imbalance spike,
///   trend flip, alignment drop) can trigger early calls, subject to a
///   separate cooldown (default 60s).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use serde_json::Value;

mod context;
mod lessons;
mod llm;
mod publisher;
mod steer;
mod subscriber;

use context::{ChangeDetector, ContextBuilder, ObserverSnapshot};
use llm::LlmClient;
use publisher::FluxPublisher;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name    = "knowledge-gene",
    about   = "LLM interpreter companion for observer-gene",
    version,
)]
struct Config {
    /// Flux Universe WebSocket URL
    #[arg(long, default_value = "wss://api.flux-universe.com/api/ws")]
    flux_ws_url: String,

    /// Flux Universe HTTP base URL (for publishing interpretations)
    #[arg(long, default_value = "https://api.flux-universe.com")]
    flux_http_url: String,

    /// Flux auth token (bearer).  Can also be set via FLUX_TOKEN env var.
    #[arg(long)]
    flux_token: Option<String>,

    /// LLM API endpoint (OpenAI-compatible).
    /// Examples: https://api.openai.com/v1  |  http://localhost:11434/v1
    #[arg(long, default_value = "https://api.openai.com/v1")]
    llm_endpoint: String,

    /// LLM model name
    #[arg(long, default_value = "gpt-4o-mini")]
    llm_model: String,

    /// Minimum seconds between LLM interpretations (budget guard)
    #[arg(long, default_value_t = 300)]
    interval_secs: u64,

    /// Minimum seconds between early (significant-change) interpretations
    #[arg(long, default_value_t = 60)]
    early_cooldown_secs: u64,

    /// State snapshot history buffer size
    #[arg(long, default_value_t = 40)]
    state_history: usize,

    /// Interpretation history buffer size
    #[arg(long, default_value_t = 10)]
    interp_history: usize,

    /// Number of most-recent state-history snapshots rendered with full cluster detail;
    /// older snapshots render in summary trajectory form.
    #[arg(long, default_value_t = 10)]
    state_detail_cap: usize,

    /// Enable steering: push suggested_actions to observer-gene.
    /// Default off (observe-only).  The steering interface is currently stubbed.
    #[arg(long, default_value_t = false)]
    steer: bool,

    /// Disable LLM reasoning/thinking mode.  Thinking is on by default;
    /// set this flag to turn it off for a run (e.g., to reduce latency).
    #[arg(long, default_value_t = false)]
    disable_thinking: bool,

    /// Maximum tokens the LLM may generate per call.
    #[arg(long, default_value_t = 12000)]
    max_tokens: u32,

    /// HTTP timeout for LLM calls in seconds.
    #[arg(long, default_value_t = 2700)]
    http_timeout_secs: u64,

    /// Symbols seen in any cluster within the last N observer ticks are kept in
    /// the active vocabulary even when not in the current cluster's closure.
    /// Default ≈24 hours at observer-gene's ~20 Hz tick rate.
    #[arg(long, default_value_t = 1_700_000)]
    vocab_recency_ticks: u64,

    /// Cap for the long-horizon state-sample buffer (sparser tier).
    #[arg(long, default_value_t = 250)]
    state_long_cap: usize,

    /// Sampling interval (in observer-gene ticks) for the long-horizon buffer.
    /// Default 6000 ≈ 5 min at observer-gene's ~20 Hz tick rate.
    /// ~250 samples × 5 min ≈ 21 h of lookback.
    #[arg(long, default_value_t = 6000)]
    state_long_interval_ticks: u64,

    /// Seconds between distillation runs (each run produces one lesson
    /// appended to knowledge-gene/lessons). Default 24 h.
    #[arg(long, default_value_t = 86400)]
    distill_interval_secs: u64,

    /// Approximate token budget for the Past Pattern Lessons section in each
    /// prompt. Oldest lessons are dropped first when budget is exceeded.
    #[arg(long, default_value_t = 8000)]
    lesson_token_budget: usize,

    /// Maximum total lessons retained in storage. When exceeded, the oldest
    /// are pruned (best-effort delete from Flux).
    #[arg(long, default_value_t = 500)]
    max_lessons: usize,

    /// max_tokens cap for the distillation LLM call. Distillation output is
    /// bounded prose and doesn't need the same headroom as interpretation.
    #[arg(long, default_value_t = 2000)]
    distill_max_tokens: u32,
}

// ── Bootstrap ─────────────────────────────────────────────────────────────────

/// Fetch the current state of static observer-gene entities via the Flux REST
/// API and pre-populate the shared state map.
///
/// Endpoint: GET /api/state/entities/{entity_id}
/// Response: {"id":"...","properties":{...}}
async fn bootstrap_entities(
    http_base: &str,
    shared:    &std::sync::Arc<std::sync::Mutex<subscriber::ObserverState>>,
) {
    let client = reqwest::Client::new();
    let entities = &[
        "observer-gene/key",
        "observer-gene/symbols",
        "observer-gene/state",
    ];

    for entity_id in entities {
        let encoded = entity_id.replace('/', "%2F");
        let url = format!("{}/api/state/entities/{}", http_base.trim_end_matches('/'), encoded);

        match client.get(&url).send().await {
            Err(e) => {
                tracing::warn!("bootstrap: GET {} failed: {}", entity_id, e);
            }
            Ok(resp) if !resp.status().is_success() => {
                tracing::warn!("bootstrap: GET {} returned {}", entity_id, resp.status());
            }
            Ok(resp) => {
                match resp.json::<serde_json::Value>().await {
                    Err(e) => {
                        tracing::warn!("bootstrap: parse error for {}: {}", entity_id, e);
                    }
                    Ok(json) => {
                        if let Some(props) = json.get("properties").and_then(|v| v.as_object()) {
                            let mut s = shared.lock().unwrap();
                            let entry = s.entities.entry(entity_id.to_string()).or_default();
                            for (k, v) in props {
                                entry.insert(k.clone(), v.clone());
                            }
                            tracing::info!(
                                "bootstrap: loaded {} ({} properties)",
                                entity_id,
                                props.len()
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Fetch the knowledge-gene/lessons entity from Flux and parse stored lessons.
///
/// A 404 is normal on first deploy — returns an empty Vec.
async fn bootstrap_lessons(http_base: &str) -> Vec<lessons::Lesson> {
    let client = reqwest::Client::new();
    let encoded = "knowledge-gene%2Flessons";
    let url = format!("{}/api/state/entities/{}", http_base.trim_end_matches('/'), encoded);

    match client.get(&url).send().await {
        Err(e) => {
            tracing::warn!("bootstrap: GET knowledge-gene/lessons failed: {}", e);
            Vec::new()
        }
        Ok(resp) => {
            let status = resp.status();
            if status.as_u16() == 404 {
                tracing::info!("bootstrap: knowledge-gene/lessons not found (first deploy) — starting with empty lessons");
                return Vec::new();
            }
            if !status.is_success() {
                tracing::warn!("bootstrap: GET knowledge-gene/lessons returned {}", status);
                return Vec::new();
            }
            match resp.json::<serde_json::Value>().await {
                Err(e) => {
                    tracing::warn!("bootstrap: parse error for knowledge-gene/lessons: {}", e);
                    Vec::new()
                }
                Ok(json) => {
                    let props = json.get("properties")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    let parsed = lessons::parse_lessons_from_properties(&props);
                    tracing::info!(
                        "bootstrap: loaded {} lessons from knowledge-gene/lessons",
                        parsed.len()
                    );
                    parsed
                }
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let cfg = Config::parse();

    let api_key   = std::env::var("KNOWLEDGE_GENE_API_KEY").ok();
    let flux_token = cfg.flux_token.clone()
        .or_else(|| std::env::var("FLUX_TOKEN").ok());

    tracing::info!("knowledge-gene starting");
    tracing::info!("  flux ws:      {}", cfg.flux_ws_url);
    tracing::info!("  flux http:    {}", cfg.flux_http_url);
    tracing::info!("  llm endpoint: {}", cfg.llm_endpoint);
    tracing::info!("  llm model:    {}", cfg.llm_model);
    tracing::info!("  llm thinking: {}", if cfg.disable_thinking { "disabled" } else { "enabled" });
    tracing::info!("  llm max_tok:  {}", cfg.max_tokens);
    tracing::info!("  llm http_to:  {}s", cfg.http_timeout_secs);
    tracing::info!("  state hist:   {} (detail cap: {})", cfg.state_history, cfg.state_detail_cap);
    tracing::info!("  state long:   {} (sample interval: {} ticks)", cfg.state_long_cap, cfg.state_long_interval_ticks);
    tracing::info!("  interp hist:  {}", cfg.interp_history);
    tracing::info!("  interval:     {}s (early cooldown: {}s)",
        cfg.interval_secs, cfg.early_cooldown_secs);
    tracing::info!("  vocab recency: {} ticks", cfg.vocab_recency_ticks);
    tracing::info!("  mode:         {}",
        if cfg.steer { "steering enabled" } else { "observe-only" });
    tracing::info!("  api_key:      {}",
        if api_key.is_some() { "set" } else { "NOT SET — LLM calls will fail without auth" });
    tracing::info!("  distill int:  {}s", cfg.distill_interval_secs);
    tracing::info!("  lesson budget: {} tokens", cfg.lesson_token_budget);
    tracing::info!("  max lessons:  {}", cfg.max_lessons);
    tracing::info!("  distill tok:  {}", cfg.distill_max_tokens);

    // Spawn the WS subscriber; returns the shared state handle.
    let shared = subscriber::spawn(cfg.flux_ws_url.clone());

    // Bootstrap static entities from the Flux REST API.
    // observer-gene/key and observer-gene/symbols are published once at
    // observer-gene startup.  New WS subscribers only receive future updates,
    // so we fetch current state via HTTP before the main loop begins.
    bootstrap_entities(&cfg.flux_http_url, &shared).await;

    // Bootstrap lessons from knowledge-gene/lessons (404 = first deploy, OK).
    let bootstrap_lesson_list = bootstrap_lessons(&cfg.flux_http_url).await;

    let publisher = FluxPublisher::new(cfg.flux_http_url.clone(), flux_token);
    let llm       = LlmClient::new(
        cfg.llm_endpoint.clone(),
        cfg.llm_model.clone(),
        api_key.clone(),
        !cfg.disable_thinking,
        cfg.max_tokens,
        cfg.http_timeout_secs,
    );
    let distill_llm = LlmClient::new(
        cfg.llm_endpoint.clone(),
        cfg.llm_model.clone(),
        api_key,
        !cfg.disable_thinking,
        cfg.distill_max_tokens,
        cfg.http_timeout_secs,
    );

    let mut ctx      = ContextBuilder::new(
        cfg.state_history,
        cfg.interp_history,
        cfg.state_detail_cap,
        cfg.vocab_recency_ticks,
        cfg.state_long_cap,
        cfg.state_long_interval_ticks,
        cfg.lesson_token_budget,
    );
    ctx.set_lessons(bootstrap_lesson_list);
    let mut detector = ChangeDetector::new();

    let interval         = Duration::from_secs(cfg.interval_secs);
    let early_cooldown   = Duration::from_secs(cfg.early_cooldown_secs);
    let distill_interval = Duration::from_secs(cfg.distill_interval_secs);

    // Start the clock before the interval so the first interpretation fires as
    // soon as we have a valid snapshot (instead of waiting a full interval).
    let mut last_interpret = Instant::now()
        .checked_sub(interval)
        .unwrap_or_else(Instant::now);

    let mut last_seen_tick:  u64  = 0;
    let mut force_interpret: bool = false;
    // Initialize to now so the first distillation runs distill_interval_secs
    // after process start, not immediately.
    let mut last_distillation = Instant::now();

    tracing::info!("main loop started — polling every 5s");

    loop {
        // ── Read current state from shared (brief lock, no await held) ────────
        let (state_opt, symbols_opt, key_opt): (
            Option<ObserverSnapshot>,
            Option<HashMap<String, Value>>,
            Option<HashMap<String, Value>>,
        ) = {
            let s = shared.lock().unwrap();
            let state = s.entities
                .get("observer-gene/state")
                .and_then(|props| ObserverSnapshot::from_props(props));
            let symbols = s.entities.get("observer-gene/symbols").cloned();
            let key     = s.entities.get("observer-gene/key").cloned();
            (state, symbols, key)
        };

        // ── Process new tick ──────────────────────────────────────────────────
        if let Some(ref snap) = state_opt {
            if snap.tick != last_seen_tick {
                tracing::debug!(
                    "new tick={} dominant={:?} imbalance={:.1} trend={}",
                    snap.tick, snap.dominant, snap.imbalance, snap.imbalance_trend
                );

                ctx.push_state(snap.clone());

                // Significant change detection (detector seeds itself on first call)
                if detector.check_and_advance(snap) {
                    let elapsed = last_interpret.elapsed();
                    if elapsed >= early_cooldown {
                        force_interpret = true;
                    } else {
                        tracing::debug!(
                            "sig change detected but early cooldown active ({:.0}s remaining)",
                            (early_cooldown - elapsed).as_secs_f32()
                        );
                    }
                }

                last_seen_tick = snap.tick;
            }
        }

        // ── Decide whether to interpret ───────────────────────────────────────
        let elapsed          = last_interpret.elapsed();
        let interval_expired = elapsed >= interval;
        let should_interpret = interval_expired || force_interpret;

        if should_interpret {
            match state_opt {
                None => {
                    tracing::debug!("no observer-gene state yet — waiting");
                }
                Some(ref snap) => {
                    tracing::info!(
                        "interpreting: tick={} dominant={:?} imbalance={:.1} ({})",
                        snap.tick,
                        snap.dominant,
                        snap.imbalance,
                        if force_interpret { "early/significant change" } else { "scheduled" },
                    );

                    match llm.interpret(snap, &ctx, &symbols_opt, &key_opt).await {
                        Ok(interp) => {
                            tracing::info!(
                                "interpretation: confidence={:.2} themes=[{}]",
                                interp.confidence,
                                interp.themes.join(", "),
                            );
                            // Log the first 200 chars of the narrative
                            let preview = if interp.interpretation.len() > 200 {
                                format!("{}...", &interp.interpretation[..200])
                            } else {
                                interp.interpretation.clone()
                            };
                            tracing::info!("narrative: {}", preview);

                            if cfg.steer {
                                steer::apply(&publisher, &interp).await;
                            }

                            detector.record_interpretation(snap);
                            ctx.push_interpretation(interp.clone());

                            if let Err(e) = publisher.publish(&interp, key_opt.as_ref()).await {
                                tracing::warn!("flux publish error: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("LLM call failed: {}", e);
                            // Don't reset last_interpret on failure so we retry
                            // after a shorter window (one polling cycle = 5s).
                            // But do clear the force flag to avoid a rapid retry loop.
                            force_interpret = false;
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    }

                    last_interpret   = Instant::now();
                    force_interpret  = false;
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;

        // ── Distillation scheduling ───────────────────────────────────────────
        if last_distillation.elapsed() >= distill_interval {
            tracing::info!(
                "distill: starting (last lesson age {} s)",
                last_distillation.elapsed().as_secs()
            );
            match lessons::distill(
                &distill_llm,
                &ctx.state_long_history,
                &ctx.interp_history,
                cfg.distill_max_tokens,
            ).await {
                Ok(lesson) => {
                    tracing::info!(
                        "distill: elapsed={}s tick_range={}..{} themes=[{}] narrative_chars={}",
                        last_distillation.elapsed().as_secs(),
                        lesson.tick_range_start,
                        lesson.tick_range_end,
                        lesson.themes.join(", "),
                        lesson.narrative.len(),
                    );
                    if let Err(e) = publisher.publish_lesson(&lesson).await {
                        tracing::warn!("distill: publish_lesson failed: {}", e);
                    }
                    ctx.push_lesson(lesson.clone());
                    // Prune oldest if over max_lessons
                    while ctx.lessons.len() > cfg.max_lessons {
                        let oldest = ctx.lessons.remove(0);
                        if let Err(e) = publisher.delete_lesson(oldest.tick_range_end).await {
                            tracing::warn!(
                                "distill: delete_lesson({}) failed: {}",
                                oldest.tick_range_end, e
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("distill failed: {}", e);
                }
            }
            last_distillation = Instant::now();
        }
    }
}
