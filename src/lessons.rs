/// Long-term pattern lessons — compressed memory for knowledge-gene.
///
/// Each Lesson is a model-distilled narrative summary of a past period,
/// persisted in Flux as `knowledge-gene/lessons` and rendered into every
/// interpretation prompt so the model can recognize returning patterns
/// across days and weeks.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};

use crate::context::{Interpretation, ObserverSnapshot};
use crate::llm::LlmClient;

// ── Lesson ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Lesson {
    pub tick_range_start: u64,
    pub tick_range_end:   u64,
    pub themes:           Vec<String>,
    pub narrative:        String,
    pub created_at_ms:    i64,
}

// ── Distillation system prompt ────────────────────────────────────────────────

pub const DISTILL_SYSTEM_PROMPT: &str = "You are distilling observer-gene's recent state activity into a compact narrative lesson, suitable for future pattern recognition by the same agent. Capture the dominant regime, key transitions, notable cross-domain correlations, and anything worth remembering for recognizing returning patterns. Be specific about which domains, signals, or symbols characterized the period. Avoid hedging language and bullet lists; aim for 200–500 characters of prose.";

// ── distill ───────────────────────────────────────────────────────────────────

/// Distill recent state and interpretation history into a compact Lesson.
///
/// Calls the LLM with a strict json_schema; sets `created_at_ms` at call time.
pub async fn distill(
    llm:                &LlmClient,
    state_long_history: &VecDeque<ObserverSnapshot>,
    interp_history:     &VecDeque<Interpretation>,
    distill_max_tokens: u32,
) -> Result<Lesson> {
    if state_long_history.is_empty() && interp_history.is_empty() {
        return Err(anyhow!("distill: no state or interpretation history available"));
    }

    // ── Build user message ────────────────────────────────────────────────────

    let mut user_msg = String::new();

    if !state_long_history.is_empty() {
        user_msg.push_str("## Recent State Activity (last ~24h, sampled every ~5 min)\n\n");
        for s in state_long_history.iter() {
            user_msg.push_str(&format!(
                "tick={} dom={} cluster_size={} imb={:.1} trend={} align={:.2}\n",
                s.tick,
                s.dominant.as_deref().unwrap_or("none"),
                s.cluster.len(),
                s.imbalance,
                s.imbalance_trend,
                s.identity_alignment,
            ));
        }
        user_msg.push('\n');
    }

    if !interp_history.is_empty() {
        user_msg.push_str("## Recent Interpretations\n\n");
        for interp in interp_history.iter() {
            let excerpt = if interp.interpretation.len() > 400 {
                format!("{}...", &interp.interpretation[..400])
            } else {
                interp.interpretation.clone()
            };
            user_msg.push_str(&format!(
                "--- tick_ref={} confidence={:.2} themes=[{}]\n{}\n\n",
                interp.tick_ref,
                interp.confidence,
                interp.themes.join(", "),
                excerpt,
            ));
        }
    }

    // ── LLM call with strict json_schema ─────────────────────────────────────

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "tick_range_start": { "type": "integer" },
            "tick_range_end":   { "type": "integer" },
            "themes":           { "type": "array", "items": { "type": "string" } },
            "narrative":        { "type": "string" },
        },
        "required": ["tick_range_start", "tick_range_end", "themes", "narrative"],
        "additionalProperties": false,
    });

    let parsed = llm.complete_with_schema(
        DISTILL_SYSTEM_PROMPT,
        &user_msg,
        "distill_response",
        schema,
        distill_max_tokens,
    ).await?;

    let tick_range_start = parsed.get("tick_range_start")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("distill: missing tick_range_start in response"))?;

    let tick_range_end = parsed.get("tick_range_end")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("distill: missing tick_range_end in response"))?;

    let themes: Vec<String> = parsed.get("themes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    let narrative = parsed.get("narrative")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let created_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    Ok(Lesson {
        tick_range_start,
        tick_range_end,
        themes,
        narrative,
        created_at_ms,
    })
}

// ── render_lessons_section ────────────────────────────────────────────────────

/// Render the Past Pattern Lessons section for inclusion in a prompt.
///
/// Greedily includes lessons newest-to-oldest within the token budget
/// (chars / 3 heuristic), then reverses so the rendered section is
/// oldest → newest. Returns the rendered string and the count emitted.
/// Returns an empty string and 0 if `lessons` is empty.
pub fn render_lessons_section(lessons: &[Lesson], budget: usize) -> (String, usize) {
    if lessons.is_empty() {
        return (String::new(), 0);
    }

    let mut included: Vec<&Lesson> = Vec::new();
    let mut token_estimate: usize = 0;

    // Iterate newest → oldest
    for lesson in lessons.iter().rev() {
        let line = format!(
            "[ticks {}..{} | created {} | themes: {}]\n{}\n\n",
            lesson.tick_range_start,
            lesson.tick_range_end,
            lesson.created_at_ms,
            lesson.themes.join(", "),
            lesson.narrative,
        );
        let cost = line.len() / 3;
        if token_estimate + cost > budget && !included.is_empty() {
            break;
        }
        token_estimate += cost;
        included.push(lesson);
    }

    let count = included.len();

    // Reverse to render oldest → newest
    included.reverse();

    let mut out = format!(
        "## Past Pattern Lessons (last {}, oldest → newest)\n\n",
        count
    );
    for lesson in included {
        out.push_str(&format!(
            "[ticks {}..{} | created {} | themes: {}]\n{}\n\n",
            lesson.tick_range_start,
            lesson.tick_range_end,
            lesson.created_at_ms,
            lesson.themes.join(", "),
            lesson.narrative,
        ));
    }

    (out, count)
}

// ── parse_lessons_from_properties ─────────────────────────────────────────────

/// Parse lessons from a Flux entity's properties map (bootstrap path).
///
/// Iterates keys starting with "lesson_", parses each value as a Lesson,
/// skips malformed entries with a warn log, and returns the result sorted
/// by tick_range_end ascending.
pub fn parse_lessons_from_properties(
    props: &serde_json::Map<String, serde_json::Value>,
) -> Vec<Lesson> {
    let mut lessons: Vec<Lesson> = Vec::new();

    for (key, value) in props {
        if !key.starts_with("lesson_") {
            continue;
        }
        match serde_json::from_value::<Lesson>(value.clone()) {
            Ok(lesson) => lessons.push(lesson),
            Err(e) => {
                tracing::warn!("parse_lessons: skipping malformed lesson key={}: {}", key, e);
            }
        }
    }

    lessons.sort_by_key(|l| l.tick_range_end);
    lessons
}
