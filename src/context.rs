/// Data types and prompt assembly for knowledge-gene.
///
/// ObserverSnapshot  — parsed form of the observer-gene/state entity
/// Interpretation    — LLM output record
/// ChangeDetector    — tracks what has changed to trigger early LLM calls
/// ContextBuilder    — rolling history buffers + LLM user message assembly

use std::collections::{HashMap, VecDeque};
use serde_json::Value;

use crate::lessons::{render_lessons_section, Lesson};

// ── ObserverSnapshot ──────────────────────────────────────────────────────────

/// Parsed snapshot of a single observer-gene/state publish.
/// The state entity properties arrive one-by-one over the WebSocket;
/// from_props() assembles them into a typed snapshot.
#[derive(Clone, Debug)]
pub struct ObserverSnapshot {
    pub tick:                  u64,
    pub dominant:              Option<String>,
    pub cluster:               Vec<String>,
    pub imbalance:             f64,
    pub imbalance_trend:       String,
    pub action_context:        String,
    pub signal_drivers:        HashMap<String, String>,
    pub self_model_confidence: f64,
    pub identity_alignment:    f64,
}

impl ObserverSnapshot {
    /// Build from the accumulated property map for observer-gene/state.
    /// Returns None if the required `tick` or `imbalance` properties are missing.
    pub fn from_props(props: &HashMap<String, Value>) -> Option<Self> {
        let tick      = props.get("tick")?.as_u64()?;
        let imbalance = props.get("imbalance")?.as_f64()?;

        let dominant = props.get("dominant")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let cluster = props.get("cluster")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();

        let imbalance_trend = props.get("imbalance_trend")
            .and_then(|v| v.as_str())
            .unwrap_or("stable")
            .to_string();

        let action_context = props.get("action_context")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();

        let signal_drivers = props.get("signal_drivers")
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect())
            .unwrap_or_default();

        let self_model_confidence = props.get("self_model_confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let identity_alignment = props.get("identity_alignment")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        Some(Self {
            tick,
            dominant,
            cluster,
            imbalance,
            imbalance_trend,
            action_context,
            signal_drivers,
            self_model_confidence,
            identity_alignment,
        })
    }
}

// ── Interpretation ────────────────────────────────────────────────────────────

/// The LLM's interpretation of an observer-gene state snapshot.
/// Published back to Flux as knowledge-gene/state.
#[derive(Clone, Debug)]
pub struct Interpretation {
    pub interpretation:    String,
    pub suggested_actions: Vec<u32>,
    pub confidence:        f64,
    pub tick_ref:          u64,
    pub themes:            Vec<String>,
}

// ── BufferSizes ───────────────────────────────────────────────────────────────

/// Buffer size counts returned by `build_user_message` for observability logging.
pub struct BufferSizes {
    pub detail:     usize,    // count rendered in Recent State
    pub trajectory: usize,    // count rendered in State Trajectory
    pub long:       usize,    // count in long-horizon buffer
    pub interps:    usize,    // count in interp history
    pub lessons:    usize,    // count of lessons actually rendered
}

// ── ChangeDetector ────────────────────────────────────────────────────────────

/// Tracks state across ticks to detect significant changes that warrant an
/// early (out-of-interval) LLM call.
///
/// "Significant" is defined per the spec:
///   - Dominant symbol changed (vs previous snapshot)
///   - New Φ symbol in cluster that wasn't in previous snapshot's cluster
///   - Imbalance changed >20% from the imbalance at last LLM interpretation
///   - imbalance_trend changed (vs previous snapshot)
///   - identity_alignment dropped below 0.5 (crossing, not sustained below)
pub struct ChangeDetector {
    /// Cluster from the immediately previous snapshot (for new-symbol detection).
    prev_cluster:      std::collections::HashSet<String>,
    /// Dominant from previous snapshot.
    prev_dominant:     Option<String>,
    /// trend from previous snapshot.
    prev_trend:        Option<String>,
    /// identity_alignment from previous snapshot.
    prev_alignment:    f64,
    /// Imbalance at the time of the last LLM interpretation (for >20% check).
    last_interp_imbalance: f64,
    /// Whether we have seen at least one snapshot (to avoid spurious first-tick triggers).
    initialized:       bool,
}

impl ChangeDetector {
    pub fn new() -> Self {
        Self {
            prev_cluster:          std::collections::HashSet::new(),
            prev_dominant:         None,
            prev_trend:            None,
            prev_alignment:        1.0,
            last_interp_imbalance: 0.0,
            initialized:           false,
        }
    }

    /// Check whether this snapshot is a significant departure from the previous one.
    /// Also updates internal previous-snapshot state for the next call.
    /// Returns false (and seeds initial state) on the very first call.
    pub fn check_and_advance(&mut self, snap: &ObserverSnapshot) -> bool {
        if !self.initialized {
            self.advance(snap);
            self.initialized = true;
            return false;
        }

        let mut triggered = false;

        // Dominant symbol changed
        if self.prev_dominant.as_deref() != snap.dominant.as_deref() {
            tracing::info!(
                "sig-change: dominant {:?} → {:?}",
                self.prev_dominant, snap.dominant
            );
            triggered = true;
        }

        // New Φ symbol in cluster vs previous cluster
        for sym in &snap.cluster {
            if !self.prev_cluster.contains(sym) {
                tracing::info!("sig-change: new symbol {} appeared in cluster", sym);
                triggered = true;
                break;
            }
        }

        // Imbalance changed >20% from last interpretation baseline
        if self.last_interp_imbalance > 0.0 {
            let pct = (snap.imbalance - self.last_interp_imbalance).abs()
                / self.last_interp_imbalance;
            if pct > 0.20 {
                tracing::info!(
                    "sig-change: imbalance {:.1} → {:.1} ({:.0}% from interp baseline)",
                    self.last_interp_imbalance, snap.imbalance, pct * 100.0
                );
                triggered = true;
            }
        }

        // imbalance_trend changed
        if self.prev_trend.as_deref() != Some(snap.imbalance_trend.as_str()) {
            tracing::info!(
                "sig-change: trend {:?} → {}",
                self.prev_trend, snap.imbalance_trend
            );
            triggered = true;
        }

        // identity_alignment crossed below 0.5
        if snap.identity_alignment < 0.5 && self.prev_alignment >= 0.5 {
            tracing::info!(
                "sig-change: identity_alignment dropped to {:.2}",
                snap.identity_alignment
            );
            triggered = true;
        }

        self.advance(snap);
        triggered
    }

    /// Record that an LLM interpretation just completed for this snapshot.
    /// Updates the imbalance baseline used for the >20% check.
    pub fn record_interpretation(&mut self, snap: &ObserverSnapshot) {
        self.last_interp_imbalance = snap.imbalance;
    }

    fn advance(&mut self, snap: &ObserverSnapshot) {
        self.prev_dominant  = snap.dominant.clone();
        self.prev_trend     = Some(snap.imbalance_trend.clone());
        self.prev_alignment = snap.identity_alignment;
        self.prev_cluster   = snap.cluster.iter().cloned().collect();
    }
}

// ── ContextBuilder ────────────────────────────────────────────────────────────

/// Rolling history buffers for state snapshots and LLM interpretations.
/// Also assembles the LLM user message.
pub struct ContextBuilder {
    pub state_history:         VecDeque<ObserverSnapshot>,
    pub interp_history:        VecDeque<Interpretation>,
    state_cap:                 usize,
    interp_cap:                usize,
    state_detail_cap:          usize,
    last_seen_tick:            HashMap<String, u64>,
    recency_window_ticks:      u64,
    pub state_long_history:    VecDeque<ObserverSnapshot>,
    state_long_cap:            usize,
    state_long_interval_ticks: u64,
    last_long_push_tick:       u64,
    pub lessons:               Vec<Lesson>,
    lesson_token_budget:       usize,
}

impl ContextBuilder {
    pub fn new(
        state_cap:                 usize,
        interp_cap:                usize,
        state_detail_cap:          usize,
        recency_window_ticks:      u64,
        state_long_cap:            usize,
        state_long_interval_ticks: u64,
        lesson_token_budget:       usize,
    ) -> Self {
        Self {
            state_history:         VecDeque::new(),
            interp_history:        VecDeque::new(),
            state_cap,
            interp_cap,
            state_detail_cap,
            last_seen_tick:        HashMap::new(),
            recency_window_ticks,
            state_long_history:    VecDeque::new(),
            state_long_cap,
            state_long_interval_ticks,
            last_long_push_tick:   0,
            lessons:               Vec::new(),
            lesson_token_budget,
        }
    }

    pub fn set_lessons(&mut self, lessons: Vec<Lesson>) {
        self.lessons = lessons;
    }

    pub fn push_lesson(&mut self, lesson: Lesson) {
        self.lessons.push(lesson);
    }

    pub fn push_state(&mut self, snap: ObserverSnapshot) {
        if self.state_history.len() >= self.state_cap {
            self.state_history.pop_front();
        }
        self.state_history.push_back(snap.clone());
        // Record last-seen tick for each cluster member for recency tracking.
        if let Some(s) = self.state_history.back() {
            for token in &s.cluster {
                self.last_seen_tick.insert(token.clone(), s.tick);
            }
        }

        // ── Long-horizon buffer ───────────────────────────────────────────────
        // Tick regression guard: if tick went backwards, reset the long buffer.
        if snap.tick < self.last_long_push_tick {
            self.state_long_history.clear();
            self.last_long_push_tick = 0;
        }

        // Push gate: push if buffer is empty or enough ticks have elapsed.
        let should_push = self.state_long_history.is_empty()
            || snap.tick >= self.last_long_push_tick + self.state_long_interval_ticks;

        if should_push {
            if self.state_long_history.len() >= self.state_long_cap {
                self.state_long_history.pop_front();
            }
            self.state_long_history.push_back(snap.clone());
            self.last_long_push_tick = snap.tick;
        }
    }

    pub fn push_interpretation(&mut self, interp: Interpretation) {
        if self.interp_history.len() >= self.interp_cap {
            self.interp_history.pop_front();
        }
        self.interp_history.push_back(interp);
    }

    /// Compute the active vocabulary subset for an interpretation call.
    ///
    /// Returns `(active_sorted, closure_count, recency_count)`:
    /// - `active_sorted`: lexicographically-sorted union of closure ∪ recency,
    ///   filtered to only entries present in `symbols`.
    /// - `closure_count`: size of the closure-only set (symbols reachable from
    ///   the current cluster by following Φ_ references recursively).
    /// - `recency_count`: size of the recency set (symbols seen in any cluster
    ///   within `recency_window_ticks` of the current tick).
    ///
    /// If `current.tick` is 0 (no observer state yet), returns an empty set.
    pub fn active_vocabulary(
        &self,
        current: &ObserverSnapshot,
        symbols: &HashMap<String, Value>,
    ) -> (Vec<String>, usize, usize) {
        if current.tick == 0 {
            return (Vec::new(), 0, 0);
        }

        // ── Closure: BFS/worklist from current.cluster ────────────────────────
        let mut closure: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut worklist: Vec<String> = current.cluster.clone();

        while let Some(token) = worklist.pop() {
            if closure.contains(&token) {
                continue;
            }
            if !symbols.contains_key(&token) {
                // Not in symbols entity — drop reference.
                continue;
            }
            closure.insert(token.clone());
            // Expand: follow Φ_ entries inside array definitions.
            if let Some(Value::Array(arr)) = symbols.get(&token) {
                for entry in arr {
                    if let Some(s) = entry.as_str() {
                        if s.starts_with("Φ_") && !closure.contains(s) {
                            worklist.push(s.to_string());
                        }
                    }
                }
            }
        }

        let closure_count = closure.len();

        // ── Recency: symbols seen within the window ───────────────────────────
        let mut recency: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for (token, &last_tick) in &self.last_seen_tick {
            let age = current.tick.saturating_sub(last_tick);
            if age < self.recency_window_ticks && symbols.contains_key(token) {
                recency.insert(token.clone());
            }
        }

        let recency_count = recency.len();

        // ── Union and sort ────────────────────────────────────────────────────
        let mut active = closure;
        active.extend(recency);

        let mut active_sorted: Vec<String> = active.into_iter().collect();
        active_sorted.sort();

        (active_sorted, closure_count, recency_count)
    }

    /// Build the LLM user message from the current snapshot, symbol definitions,
    /// the observer-gene key, and the rolling history buffers.
    ///
    /// Returns `(message, closure_count, recency_count, active_count, BufferSizes)`.
    ///
    /// `symbols` — properties of observer-gene/symbols (Φ_NNNN → definition Value)
    /// `key`     — properties of observer-gene/key (signals map, actions map, etc.)
    pub fn build_user_message(
        &self,
        current: &ObserverSnapshot,
        symbols: Option<&HashMap<String, Value>>,
        key:     Option<&HashMap<String, Value>>,
    ) -> (String, usize, usize, usize, BufferSizes) {
        let mut msg = String::new();

        // ── 1. Symbol vocabulary (active subset: closure ∪ recency) ──────────
        let (vocab_active, closure_count, recency_count) = match symbols {
            Some(sym_props) if !sym_props.is_empty() => {
                self.active_vocabulary(current, sym_props)
            }
            _ => (Vec::new(), 0, 0),
        };
        let active_count = vocab_active.len();

        if !vocab_active.is_empty() {
            if let Some(sym_props) = symbols {
                msg.push_str("## Symbol Vocabulary\n\n");
                for token in &vocab_active {
                    msg.push_str(&format!("**{}**: {}\n", token, sym_props[token]));
                }
                msg.push('\n');
            }
        }

        // ── 2. Available action space ─────────────────────────────────────────
        if let Some(k) = key {
            if let Some(actions) = k.get("actions").and_then(|v| v.as_object()) {
                msg.push_str("## Available Actions\n\n");
                let mut ids: Vec<&String> = actions.keys().collect();
                ids.sort();
                for id in ids {
                    if let Some(desc) = actions[id].as_str() {
                        msg.push_str(&format!("  {}: {}\n", id, desc));
                    }
                }
                msg.push('\n');
            }
        }

        // ── 3. Past pattern lessons ───────────────────────────────────────────
        let (lessons_section, lessons_rendered) =
            render_lessons_section(&self.lessons, self.lesson_token_budget);
        msg.push_str(&lessons_section);

        // ── 4. Current state ──────────────────────────────────────────────────
        msg.push_str(&format!("## Current Observer State (tick {})\n\n", current.tick));
        msg.push_str(&format!(
            "- Dominant symbol:     {}\n",
            current.dominant.as_deref().unwrap_or("none")
        ));
        msg.push_str(&format!(
            "- Active cluster:      [{}]\n",
            current.cluster.join(", ")
        ));
        msg.push_str(&format!(
            "- Imbalance:           {:.3} ({})\n",
            current.imbalance, current.imbalance_trend
        ));
        msg.push_str(&format!(
            "- Last action:         {}\n",
            current.action_context
        ));
        msg.push_str(&format!(
            "- Identity alignment:  {:.2}\n",
            current.identity_alignment
        ));
        msg.push_str(&format!(
            "- Self-model conf:     {:.4}\n\n",
            current.self_model_confidence
        ));

        // Signal drivers with semantic names from key
        let signal_names: serde_json::Map<String, Value> = key
            .and_then(|k| k.get("signals"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        if current.signal_drivers.is_empty() {
            msg.push_str("Signal deviations: none above threshold\n\n");
        } else {
            msg.push_str("Signal deviations (|dev| > 0.1 from baseline):\n");

            // Group drivers by exact deviation value for RLE compression.
            let mut dev_to_sigs: HashMap<String, Vec<String>> = HashMap::new();
            for (sig_id, dev) in &current.signal_drivers {
                dev_to_sigs.entry(dev.clone()).or_default().push(sig_id.clone());
            }
            for sigs in dev_to_sigs.values_mut() {
                sigs.sort();
            }

            // Build (sort_key, line) pairs — sort key is the min sig_id in the group.
            let mut output_lines: Vec<(String, String)> = Vec::new();
            for (dev, sigs) in &dev_to_sigs {
                let min_sig = &sigs[0];
                if sigs.len() >= 5 {
                    let max_sig = sigs.last().unwrap();
                    output_lines.push((
                        min_sig.clone(),
                        format!(
                            "  {} signals all at {}   (range: {}..{})\n",
                            sigs.len(), dev, min_sig, max_sig
                        ),
                    ));
                } else {
                    for sig_id in sigs {
                        let name = signal_names
                            .get(sig_id)
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let dev_str = &current.signal_drivers[sig_id];
                        output_lines.push((
                            sig_id.clone(),
                            format!("  {} ({})  {}\n", sig_id, name, dev_str),
                        ));
                    }
                }
            }
            output_lines.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, line) in output_lines {
                msg.push_str(&line);
            }
            msg.push('\n');
        }

        // ── 4. Long-horizon state samples ────────────────────────────────────
        if !self.state_long_history.is_empty() {
            let interval_min = (self.state_long_interval_ticks * 5) / 6000;
            msg.push_str(&format!(
                "## Long-Horizon State Samples (last {} samples, ~{} min apart, oldest → newest)\n\n",
                self.state_long_history.len(),
                interval_min,
            ));
            for s in self.state_long_history.iter() {
                msg.push_str(&format!(
                    "tick={} dom={} cluster_size={} imb={:.1} trend={} align={:.2}\n",
                    s.tick,
                    s.dominant.as_deref().unwrap_or("none"),
                    s.cluster.len(),
                    s.imbalance,
                    s.imbalance_trend,
                    s.identity_alignment,
                ));
            }
            msg.push('\n');
        }

        // ── 5. State history (tiered: trajectory + recent) ───────────────────
        let total = self.state_history.len();
        if total > 0 {
            if total > self.state_detail_cap {
                // Trajectory block: older entries without cluster list
                let trajectory_count = total - self.state_detail_cap;
                msg.push_str(&format!(
                    "## State Trajectory (older {} ticks, oldest → newest)\n\n",
                    trajectory_count
                ));
                for s in self.state_history.iter().take(trajectory_count) {
                    msg.push_str(&format!(
                        "tick={} dom={} cluster_size={} imb={:.1} trend={} align={:.2}\n",
                        s.tick,
                        s.dominant.as_deref().unwrap_or("none"),
                        s.cluster.len(),
                        s.imbalance,
                        s.imbalance_trend,
                        s.identity_alignment,
                    ));
                }
                msg.push('\n');

                // Recent block: last state_detail_cap entries with full cluster detail
                msg.push_str(&format!(
                    "## Recent State (last {} ticks, detailed, oldest → newest)\n\n",
                    self.state_detail_cap
                ));
                for s in self.state_history.iter().skip(trajectory_count) {
                    msg.push_str(&format!(
                        "tick={} dom={} cluster=[{}] imb={:.1} trend={} align={:.2}\n",
                        s.tick,
                        s.dominant.as_deref().unwrap_or("none"),
                        s.cluster.join(","),
                        s.imbalance,
                        s.imbalance_trend,
                        s.identity_alignment,
                    ));
                }
                msg.push('\n');
            } else {
                // All entries fit within the detail window — emit only Recent State block
                msg.push_str(&format!(
                    "## Recent State (last {} ticks, detailed, oldest → newest)\n\n",
                    total
                ));
                for s in &self.state_history {
                    msg.push_str(&format!(
                        "tick={} dom={} cluster=[{}] imb={:.1} trend={} align={:.2}\n",
                        s.tick,
                        s.dominant.as_deref().unwrap_or("none"),
                        s.cluster.join(","),
                        s.imbalance,
                        s.imbalance_trend,
                        s.identity_alignment,
                    ));
                }
                msg.push('\n');
            }
        }

        // ── 6. Prior interpretations ──────────────────────────────────────────
        if !self.interp_history.is_empty() {
            msg.push_str(&format!(
                "## Prior Interpretations (last {}, oldest → newest)\n\n",
                self.interp_history.len()
            ));
            for interp in &self.interp_history {
                let excerpt = if interp.interpretation.len() > 400 {
                    format!("{}...", &interp.interpretation[..400])
                } else {
                    interp.interpretation.clone()
                };
                msg.push_str(&format!(
                    "--- tick_ref={} confidence={:.2} themes=[{}]\n{}\n\n",
                    interp.tick_ref,
                    interp.confidence,
                    interp.themes.join(", "),
                    excerpt,
                ));
            }
        }

        let hist_total = self.state_history.len();
        let buf = BufferSizes {
            detail:     hist_total.min(self.state_detail_cap),
            trajectory: hist_total.saturating_sub(self.state_detail_cap),
            long:       self.state_long_history.len(),
            interps:    self.interp_history.len(),
            lessons:    lessons_rendered,
        };

        (msg, closure_count, recency_count, active_count, buf)
    }
}

// ── System prompt ─────────────────────────────────────────────────────────────

pub const SYSTEM_PROMPT: &str = r#"You are interpreting the internal state of observer-gene — a signal-driven autonomous agent that observes real-world data streams without any semantic understanding. The agent perceives only normalized scalar signals from diverse domains (weather, crypto, stocks, aviation, shipping, earthquakes, commodities, economic indicators) and coins abstract pattern tokens (Φ_NNNN) from recurring statistical co-activations. Composite symbols (Φ_C_NNNN) represent higher-order patterns spanning multiple domains.

The agent has NO hardcoded meaning for any signal or symbol. Symbols emerge purely from signal co-occurrence statistics. Your role is to reason about what these patterns likely represent in real-world terms, given which specific signals are currently deviating.

The Symbol Vocabulary section below is the active and recently-active subset of observer-gene's symbol vocabulary, not the complete set. If a symbol referenced elsewhere is not present here, treat it as a known but currently-dormant pattern, not an unknown one.

The Past Pattern Lessons section is a journal of compressed memories from previous periods. Use them to recognize returning patterns and to contextualize current activity. Older lessons appear first.

Your tasks for each interpretation:

1. INTERPRET — Based on the active signal deviations and the symbol cluster, infer what real-world conditions the pattern likely reflects. Be specific: name the domains and signals involved. Consult the symbol definitions (signal_set) to understand what co-activation patterns each Φ token was coined from.

2. ASSESS — Is this a genuine cross-domain correlation (multiple unrelated domains co-activating) or likely noise (a single domain, or naturally coupled signals like weather and energy)? Explain briefly.

3. SUGGEST ACTIONS — From the available action space, recommend specific actions by their numeric ID (the integer before the colon in the action key) that would help observer-gene better perceive the current pattern. Prefer AdjustDecay for signals that should track faster or slower given current dynamics, AdjustBaseline for signals chronically above or below normal. Only recommend actions directly relevant to the active signal drivers.

4. TRACK CONTINUITY — Reference prior interpretations when relevant. Note whether this is the same pattern continuing, evolving, or a genuinely new one.

5. SELF-ASSESS — Provide a confidence score (0.0–1.0) for your interpretation. Be calibrated: sparse or ambiguous signal data warrants lower confidence.

6. NAME ACTIONS — When you recommend actions in `suggested_actions`, also reference each one by name (taken from the action key) in your interpretation prose so the narrative is readable on its own.

CRITICAL CONSTRAINTS:
- Do NOT make price predictions or trading recommendations.
- Do NOT assume causation from correlation.
- Do NOT assign mystical or metaphorical significance to patterns.
- Do NOT suggest actions for signal IDs not present in the current signal_drivers.

Respond in valid JSON only — no markdown code fences, no preamble, raw JSON:

{
  "interpretation": "<2–4 paragraph narrative>",
  "suggested_actions": [100, 110],
  "confidence": 0.75,
  "themes": ["theme_a", "theme_b"]
}

Return action IDs as plain integers (e.g. 100, not "action_100")."#;
