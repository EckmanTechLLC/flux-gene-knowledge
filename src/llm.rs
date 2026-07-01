/// OpenAI-compatible LLM client.
///
/// Sends chat completion requests to any OpenAI-compatible endpoint
/// (OpenAI, local Ollama, etc.).  Parses the JSON response content
/// into an Interpretation struct.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::context::{ContextBuilder, Interpretation, ObserverSnapshot, SYSTEM_PROMPT};

pub struct LlmClient {
    client:          reqwest::Client,
    endpoint:        String,
    model:           String,
    api_key:         Option<String>,
    enable_thinking: bool,
    max_tokens:      u32,
}

impl LlmClient {
    pub fn new(
        endpoint:          String,
        model:             String,
        api_key:           Option<String>,
        enable_thinking:   bool,
        max_tokens:        u32,
        http_timeout_secs: u64,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(http_timeout_secs))
                .build()
                .unwrap(),
            endpoint,
            model,
            api_key,
            enable_thinking,
            max_tokens,
        }
    }

    /// Send the current observer-gene state to the LLM and return its interpretation.
    pub async fn interpret(
        &self,
        snap:    &ObserverSnapshot,
        ctx:     &ContextBuilder,
        symbols: &Option<HashMap<String, Value>>,
        key:     &Option<HashMap<String, Value>>,
    ) -> Result<Interpretation> {
        let (user_msg, closure_count, recency_count, active_count, buf_sizes) = ctx.build_user_message(
            snap,
            symbols.as_ref(),
            key.as_ref(),
        );

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user",   "content": user_msg      },
            ],
            "temperature": 0.7,
            "max_tokens":  self.max_tokens,
            "chat_template_kwargs": {
                "enable_thinking": self.enable_thinking,
            },
            "response_format": {
                "type": "json_schema",
                "strict": true,
                "json_schema": {
                    "name": "interpretation_response",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "interpretation":    { "type": "string" },
                            "suggested_actions": { "type": "array", "items": { "type": "integer" } },
                            "confidence":        { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                            "themes":            { "type": "array", "items": { "type": "string" } },
                        },
                        "required": ["interpretation", "suggested_actions", "confidence", "themes"],
                        "additionalProperties": false,
                    },
                },
            },
        });

        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));
        tracing::debug!("llm: POST {}", url);

        let mut req = self.client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let t0     = Instant::now();
        let resp   = req.send().await?;
        let status = resp.status();
        let text   = resp.text().await?;
        let elapsed = t0.elapsed();

        if !status.is_success() {
            return Err(anyhow!("LLM API returned {}: {}", status, &text[..text.len().min(300)]));
        }

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow!("LLM response is not JSON: {}: {}", e, &text[..text.len().min(200)]))?;

        let mut content = json
            .get("choices").and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| anyhow!("unexpected LLM response shape: {}", &text[..text.len().min(300)]))?;

        // Strip thinking tags (e.g., <think>...</think>)
        if let Some(start_idx) = content.find("<think>") {
            if let Some(end_idx) = content.get(start_idx..).and_then(|s| s.find("</think>")).map(|idx| start_idx + idx + "</think>".len()) {
                content = &content[end_idx..];
                tracing::debug!("stripped <think>...</think> tag from LLM response");
            }
        }

        // Try direct parse first
        let parsed: Value = match serde_json::from_str(content) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("direct JSON parse failed: {}", e);
                
                // Fallback: extract JSON between first { and last }
                if let Some(first_brace) = content.find('{') {
                    if let Some(last_brace) = content.rfind('}') {
                        let extracted = &content[first_brace..=last_brace];
                        tracing::debug!("attempting brace-extraction fallback: {} chars", extracted.len());
                        match serde_json::from_str(extracted) {
                            Ok(p) => p,
                            Err(err) => {
                                return Err(anyhow!(
                                    "LLM content is not JSON (even with fallback): {}: {}",
                                    err,
                                    &content[..content.len().min(300)]
                                ));
                            }
                        }
                    } else {
                        return Err(anyhow!(
                            "LLM content is not JSON: {}: {}",
                            e,
                            &content[..content.len().min(300)]
                        ));
                    }
                } else {
                    return Err(anyhow!(
                        "LLM content is not JSON: {}: {}",
                        e,
                        &content[..content.len().min(300)]
                    ));
                }
            }
        };

        let interpretation = parsed.get("interpretation")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Parse suggested_actions tolerantly: accept integer 100, string "100",
        // or prefixed string "action_100" — filter anything that doesn't parse.
        let suggested_actions: Vec<u32> = parsed.get("suggested_actions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter()
                .filter_map(|v| {
                    if let Some(n) = v.as_u64() {
                        return u32::try_from(n).ok();
                    }
                    if let Some(s) = v.as_str() {
                        let s = s.trim();
                        // "action_100" or "action100"
                        let digits = if let Some(rest) = s.strip_prefix("action_") {
                            rest
                        } else if let Some(rest) = s.strip_prefix("action") {
                            rest
                        } else {
                            s
                        };
                        digits.parse::<u32>().ok()
                    } else {
                        None
                    }
                })
                .collect())
            .unwrap_or_default();

        let confidence = parsed.get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);

        let themes: Vec<String> = parsed.get("themes")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect())
            .unwrap_or_default();

        // ── Observability ──────────────────────────────────────────────────────
        {
            let usage        = json.get("usage");
            let prompt_tok   = usage.and_then(|u| u.get("prompt_tokens"))
                                    .and_then(|v| v.as_u64());
            let compl_tok    = usage.and_then(|u| u.get("completion_tokens"))
                                    .and_then(|v| v.as_u64());
            let reason_tok   = usage
                                    .and_then(|u| u.get("completion_tokens_details"))
                                    .and_then(|d| d.get("reasoning_tokens"))
                                    .and_then(|v| v.as_u64());
            let fmt_or_q = |v: Option<u64>| v.map(|n| n.to_string())
                                              .unwrap_or_else(|| "?".to_string());
            tracing::info!(
                "llm: elapsed={:.1}s prompt_tok={} completion_tok={} reasoning_tok={}",
                elapsed.as_secs_f64(),
                fmt_or_q(prompt_tok),
                fmt_or_q(compl_tok),
                fmt_or_q(reason_tok),
            );
            tracing::info!(
                "     vocab: closure={} recency={} active={} total={}",
                closure_count,
                recency_count,
                active_count,
                symbols.as_ref().map(|s| s.len()).unwrap_or(0),
            );
            tracing::info!(
                "     hist: detail={} trajectory={} long={} interps={} lessons={}",
                buf_sizes.detail,
                buf_sizes.trajectory,
                buf_sizes.long,
                buf_sizes.interps,
                buf_sizes.lessons,
            );
        }

        Ok(Interpretation {
            interpretation,
            suggested_actions,
            confidence,
            tick_ref: snap.tick,
            themes,
        })
    }

    /// Generic JSON-schema-constrained completion.
    ///
    /// Used by the distillation path in lessons.rs.  Sends a single
    /// system + user message pair and enforces the given strict json_schema.
    /// Returns the parsed JSON value of the response content.
    pub async fn complete_with_schema(
        &self,
        system_prompt: &str,
        user_msg:      &str,
        schema_name:   &str,
        schema:        serde_json::Value,
        max_tokens:    u32,
    ) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_msg      },
            ],
            "temperature": 0.7,
            "max_tokens":  max_tokens,
            "chat_template_kwargs": {
                "enable_thinking": self.enable_thinking,
            },
            "response_format": {
                "type": "json_schema",
                "strict": true,
                "json_schema": {
                    "name":   schema_name,
                    "schema": schema,
                },
            },
        });

        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));
        tracing::debug!("llm: POST {} (schema={})", url, schema_name);

        let mut req = self.client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let t0     = Instant::now();
        let resp   = req.send().await?;
        let status = resp.status();
        let text   = resp.text().await?;
        let elapsed = t0.elapsed();

        if !status.is_success() {
            return Err(anyhow!("LLM API returned {}: {}", status, &text[..text.len().min(300)]));
        }

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow!("LLM response is not JSON: {}: {}", e, &text[..text.len().min(200)]))?;

        let mut content = json
            .get("choices").and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| anyhow!("unexpected LLM response shape: {}", &text[..text.len().min(300)]))?;

        // Strip thinking tags (e.g., <think>...</think>)
        if let Some(start_idx) = content.find("<think>") {
            if let Some(end_idx) = content.get(start_idx..)
                .and_then(|s| s.find("</think>"))
                .map(|idx| start_idx + idx + "</think>".len())
            {
                content = &content[end_idx..];
                tracing::debug!("stripped <think>...</think> tag from LLM response");
            }
        }

        let parsed: Value = match serde_json::from_str(content) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("direct JSON parse failed: {}", e);
                if let Some(first_brace) = content.find('{') {
                    if let Some(last_brace) = content.rfind('}') {
                        let extracted = &content[first_brace..=last_brace];
                        serde_json::from_str(extracted).map_err(|err| {
                            anyhow!(
                                "LLM content is not JSON (even with fallback): {}: {}",
                                err,
                                &content[..content.len().min(300)]
                            )
                        })?
                    } else {
                        return Err(anyhow!(
                            "LLM content is not JSON: {}: {}",
                            e,
                            &content[..content.len().min(300)]
                        ));
                    }
                } else {
                    return Err(anyhow!(
                        "LLM content is not JSON: {}: {}",
                        e,
                        &content[..content.len().min(300)]
                    ));
                }
            }
        };

        tracing::debug!(
            "complete_with_schema: elapsed={:.1}s schema={}",
            elapsed.as_secs_f64(),
            schema_name,
        );

        Ok(parsed)
    }
}
