//! Codex remote compaction v2 synthesis helpers.
//!
//! Non-OpenAI upstreams often answer a `compaction_trigger` turn with ordinary
//! `message` / `reasoning` items. Codex remote compaction v2 requires exactly
//! one `{ type: "compaction", encrypted_content }` output item. When a provider
//! enables synthesis, Aether wraps the upstream summary text in a self-describing
//! envelope under `encrypted_content` and restores that summary on later turns.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Map, Value};

/// Prefix for Aether-synthesized compaction envelopes (not OpenAI ciphertext).
pub const AETHER_COMPACT_SYNTHESIS_PREFIX: &str = "aether_compact_v1:";

const CONVERSATION_SUMMARY_HEADING: &str = "[Conversation Summary]";

/// Codex local compaction summarization prompt.
///
/// Source: `codex-rs/prompts/templates/compact/prompt.md`
/// (`codex_prompts::SUMMARIZATION_PROMPT` via `include_str!`). Embedded here so
/// Aether does not depend on the Codex repo at runtime.
///
/// Injected as a Responses `input` user message — same as Codex
/// `core/src/compact.rs` (`run_inline_auto_compact_task` / `UserInput::Text`),
/// not written into `instructions`.
pub const COMPACT_SUMMARIZATION_PROMPT: &str = r#"You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.
"#;

/// True when `body` is a remote-compaction request (legacy compact format or
/// v2 `compaction_trigger` input item). Does **not** treat a replayed
/// `type: "compaction"` history item as a trigger.
pub fn responses_body_is_remote_compaction_request(api_format: &str, body: &Value) -> bool {
    super::openai_responses_request_operation(api_format, body)
        == Some(super::OPENAI_RESPONSES_OPERATION_COMPACT)
}

/// Encode a plaintext summary into an `encrypted_content` carrier Codex can
/// round-trip through later requests.
pub fn encode_compact_synthesis_encrypted_content(summary: &str, model: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let payload = json!({
        "summary": summary,
        "model": model,
        "ts": ts,
    });
    format!(
        "{AETHER_COMPACT_SYNTHESIS_PREFIX}{}",
        STANDARD.encode(payload.to_string().as_bytes())
    )
}

/// Decode an Aether synthesis envelope. Returns `None` for native OpenAI blobs.
pub fn decode_compact_synthesis_encrypted_content(encrypted_content: &str) -> Option<String> {
    let encoded = encrypted_content.strip_prefix(AETHER_COMPACT_SYNTHESIS_PREFIX)?;
    let bytes = STANDARD.decode(encoded).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(str::to_string)
}

pub fn responses_output_has_compaction_item(output: &[Value]) -> bool {
    output.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("compaction")
            && item
                .get("encrypted_content")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
    })
}

/// Collect assistant-visible text from a Responses-style `output` array.
pub fn extract_summary_text_from_responses_output(output: &[Value]) -> String {
    let mut parts = Vec::new();
    for item in output {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        match item_type {
            "message" => collect_message_texts(item, &mut parts),
            "reasoning" => collect_reasoning_texts(item, &mut parts),
            _ => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    parts.join("\n\n")
}

fn collect_message_texts(item: &Value, parts: &mut Vec<String>) {
    let Some(content) = item.get("content").and_then(Value::as_array) else {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
        return;
    };
    for part in content {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(part_type, "output_text" | "text") {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }
}

fn collect_reasoning_texts(item: &Value, parts: &mut Vec<String>) {
    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
        for part in summary {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }
}

fn build_compaction_output_item(encrypted_content: &str, response_id: &str) -> Value {
    json!({
        "id": format!("{response_id}_compact"),
        "type": "compaction",
        "encrypted_content": encrypted_content,
    })
}

/// When synthesis is enabled for a compact operation and the converted client
/// body has no native compaction item, replace `output` with exactly one
/// synthesized compaction item. Returns whether a rewrite happened.
pub fn synthesize_compaction_client_response(
    client_body: &mut Value,
    model: &str,
    synthesis_enabled: bool,
    is_compact_operation: bool,
) -> Result<bool, String> {
    if !synthesis_enabled || !is_compact_operation {
        return Ok(false);
    }
    let Some(object) = client_body.as_object_mut() else {
        return Ok(false);
    };
    let output = object
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if responses_output_has_compaction_item(&output) {
        return Ok(false);
    }
    let summary = extract_summary_text_from_responses_output(&output);
    if summary.trim().is_empty() {
        return Err("compact synthesis produced an empty summary".to_string());
    }
    let response_id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("resp_compact")
        .to_string();
    let encrypted_content = encode_compact_synthesis_encrypted_content(&summary, model);
    object.insert(
        "output".to_string(),
        Value::Array(vec![build_compaction_output_item(
            &encrypted_content,
            &response_id,
        )]),
    );
    // v2 clients expect a normal response object with a compaction item.
    // Preserve legacy `response.compaction` when already set.
    if object
        .get("object")
        .and_then(Value::as_str)
        .is_none_or(|value| value == "response")
    {
        object.insert("object".to_string(), Value::String("response".to_string()));
        object.insert(
            "status".to_string(),
            Value::String("completed".to_string()),
        );
    }
    Ok(true)
}

/// Prepare a client Responses body for compact synthesis **before** cross-format
/// conversion.
///
/// When synthesis is enabled for a compact turn this:
/// 1. expands `aether_compact_v1` history into `instructions`
/// 2. strips `compaction_trigger` / residual `compaction` control items
/// 3. appends the Codex local-compaction summarization prompt as a user message
///    (aligned with `codex-rs/core/src/compact.rs`, not `instructions`)
/// 4. strips tools for tool-free summarization (Codex compact `Prompt` defaults
///    to empty tools)
///
/// History expansion still runs when synthesis is off so previously synthesized
/// envelopes remain readable on Responses replay.
///
/// Returns whether the body was mutated.
pub fn prepare_compact_synthesis_provider_request(
    body: &mut Value,
    synthesis_enabled: bool,
    is_compact_operation: bool,
) -> bool {
    let mut changed = expand_synthetic_compaction_history(body);
    if synthesis_enabled && is_compact_operation {
        if strip_compact_control_items(body) {
            changed = true;
        }
        if ensure_compact_summarization_user_message(body) {
            changed = true;
        }
        if strip_tools_for_compact_summarization(body) {
            changed = true;
        }
    }
    changed
}

fn expand_synthetic_compaction_history(body: &mut Value) -> bool {
    let Some(object) = body.as_object_mut() else {
        return false;
    };
    let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };

    let mut summaries = Vec::new();
    let mut retained = Vec::with_capacity(input.len());
    for item in input.drain(..) {
        let decoded = item
            .get("type")
            .and_then(Value::as_str)
            .filter(|item_type| *item_type == "compaction")
            .and_then(|_| item.get("encrypted_content").and_then(Value::as_str))
            .and_then(decode_compact_synthesis_encrypted_content);
        if let Some(summary) = decoded {
            summaries.push(summary);
        } else {
            retained.push(item);
        }
    }
    if summaries.is_empty() {
        *input = retained;
        return false;
    }

    *input = retained;
    let block = format!(
        "{CONVERSATION_SUMMARY_HEADING}\n{}",
        summaries.join("\n\n")
    );
    match object.get_mut("instructions") {
        Some(Value::String(existing)) if !existing.trim().is_empty() => {
            *existing = format!("{block}\n\n{existing}");
        }
        _ => {
            object.insert("instructions".to_string(), Value::String(block));
        }
    }
    true
}

fn strip_tools_for_compact_summarization(body: &mut Value) -> bool {
    let Some(object) = body.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    for key in ["tools", "tool_choice", "functions", "function_call"] {
        if object.remove(key).is_some() {
            changed = true;
        }
    }
    changed
}

/// Remove OpenAI-only compact control items that fail lossy cross-format
/// validation (`compaction_trigger`) and residual `compaction` history items
/// that were not expanded into instructions (native OpenAI blobs).
fn strip_compact_control_items(body: &mut Value) -> bool {
    let Some(object) = body.as_object_mut() else {
        return false;
    };
    let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = input.len();
    input.retain(|item| {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        item_type != "compaction_trigger" && item_type != "compaction"
    });
    before != input.len()
}

/// Append Codex's local-compaction summarization prompt as the last user
/// message in `input`, matching `run_inline_auto_compact_task` in Codex
/// `core/src/compact.rs` (prompt as `UserInput::Text`, not Responses `instructions`).
fn ensure_compact_summarization_user_message(body: &mut Value) -> bool {
    let Some(object) = body.as_object_mut() else {
        return false;
    };
    let input = object
        .entry("input".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(input) = input.as_array_mut() else {
        return false;
    };
    if input
        .iter()
        .any(input_item_contains_compact_summarization_prompt)
    {
        return false;
    }
    input.push(json!({
        "type": "message",
        "role": "user",
        "content": [{
            "type": "input_text",
            "text": COMPACT_SUMMARIZATION_PROMPT
        }]
    }));
    true
}

fn input_item_contains_compact_summarization_prompt(item: &Value) -> bool {
    match item.get("content") {
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("CONTEXT CHECKPOINT COMPACTION"))
        }),
        Some(Value::String(text)) => text.contains("CONTEXT CHECKPOINT COMPACTION"),
        _ => item
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("CONTEXT CHECKPOINT COMPACTION")),
    }
}

/// Build a minimal SSE body for a synthesized compaction turn (v2 clients).
pub fn build_compact_synthesis_sse(
    response_id: &str,
    model: &str,
    encrypted_content: &str,
) -> String {
    let compaction_item = build_compaction_output_item(encrypted_content, response_id);
    let created = json!({
        "type": "response.created",
        "response": {
            "id": response_id,
            "object": "response",
            "model": model,
            "status": "in_progress",
            "output": []
        }
    });
    let added = json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {
            "id": compaction_item["id"].clone(),
            "type": "compaction",
            "status": "in_progress"
        }
    });
    let done = json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": compaction_item
    });
    let completed = json!({
        "type": "response.completed",
        "response": {
            "id": response_id,
            "object": "response",
            "model": model,
            "status": "completed",
            "output": [compaction_item.clone()],
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0
            }
        }
    });
    [
        format!("event: response.created\ndata: {created}\n\n"),
        format!("event: response.output_item.added\ndata: {added}\n\n"),
        format!("event: response.output_item.done\ndata: {done}\n\n"),
        format!("event: response.completed\ndata: {completed}\n\n"),
    ]
    .concat()
}

/// Rewrite a completed Responses SSE stream that lacks compaction items into
/// an atomic synthesized compaction stream.
pub fn maybe_rewrite_responses_sse_for_compact_synthesis(
    sse_body: &str,
    model: &str,
    synthesis_enabled: bool,
    is_compact_operation: bool,
) -> Result<Option<String>, String> {
    if !synthesis_enabled || !is_compact_operation {
        return Ok(None);
    }
        if responses_sse_has_compaction_item(sse_body) {
        return Ok(None);
    }

    let summary = match crate::formats::shared::sync_products::aggregate_openai_responses_stream_sync_response(
        sse_body.as_bytes(),
    ) {
        Some(aggregated) => {
            let output = aggregated
                .get("output")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let text = extract_summary_text_from_responses_output(&output);
            if text.trim().is_empty() {
                extract_summary_text_from_sse(sse_body)
            } else {
                text
            }
        }
        None => extract_summary_text_from_sse(sse_body),
    };
    if summary.trim().is_empty() {
        return Err("compact synthesis produced an empty summary from stream".to_string());
    }
    let response_id =
        extract_response_id_from_sse(sse_body).unwrap_or_else(|| "resp_compact".to_string());
    let encrypted = encode_compact_synthesis_encrypted_content(&summary, model);
    Ok(Some(build_compact_synthesis_sse(
        &response_id,
        model,
        &encrypted,
    )))
}


fn responses_sse_has_compaction_item(sse_body: &str) -> bool {
    for data in iter_sse_json_payloads(sse_body) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            == Some("compaction")
        {
            return true;
        }
        if let Some(output) = value
            .pointer("/response/output")
            .and_then(Value::as_array)
        {
            if responses_output_has_compaction_item(output) {
                return true;
            }
        }
        if let Some(output) = value.get("output").and_then(Value::as_array) {
            if responses_output_has_compaction_item(output) {
                return true;
            }
        }
    }
    false
}

fn iter_sse_json_payloads(sse_body: &str) -> impl Iterator<Item = &str> {
    sse_body.lines().filter_map(|line| {
        let data = line.strip_prefix("data:")?.trim();
        if data.is_empty() || data == "[DONE]" {
            None
        } else {
            Some(data)
        }
    })
}

fn extract_summary_text_from_sse(sse_body: &str) -> String {
    let mut texts = Vec::new();
    for line in sse_body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type == "response.output_text.delta" {
            if let Some(delta) = value.get("delta").and_then(|delta| {
                delta
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| delta.get("text").and_then(Value::as_str).map(str::to_string))
            }) {
                texts.push(delta);
            }
        } else if event_type == "response.output_text.done" {
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                return text.to_string();
            }
        } else if event_type == "response.output_item.done" {
            if let Some(item) = value.get("item") {
                let mut parts = Vec::new();
                collect_message_texts(item, &mut parts);
                collect_reasoning_texts(item, &mut parts);
                if !parts.is_empty() {
                    texts.extend(parts);
                }
            }
        }
    }
    texts.join("")
}

fn extract_response_id_from_sse(sse_body: &str) -> Option<String> {
    for line in sse_body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        if let Some(id) = value
            .pointer("/response/id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            return Some(id.to_string());
        }
    }
    None
}

/// Insert synthesis flags into a report context map for finalize/stream hooks.
pub fn insert_compact_synthesis_report_context_fields(
    report_context: &mut Map<String, Value>,
    enabled: bool,
    is_compact_operation: bool,
) {
    report_context.insert(
        "codex_compact_synthesis_enabled".to_string(),
        Value::Bool(enabled),
    );
    if is_compact_operation {
        report_context.insert(
            "openai_responses_operation".to_string(),
            Value::String("compact".to_string()),
        );
    }
}

pub fn compact_synthesis_enabled_from_report_context(report_context: Option<&Value>) -> bool {
    report_context
        .and_then(|context| context.get("codex_compact_synthesis_enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn is_compact_operation_from_report_context(report_context: Option<&Value>) -> bool {
    report_context
        .and_then(|context| context.get("openai_responses_operation"))
        .and_then(Value::as_str)
        .is_some_and(|operation| operation.eq_ignore_ascii_case("compact"))
        || report_context
            .and_then(|context| context.get("client_api_format"))
            .and_then(Value::as_str)
            .is_some_and(crate::is_openai_responses_compact_format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let encoded = encode_compact_synthesis_encrypted_content("hello summary", "gpt-test");
        assert!(encoded.starts_with(AETHER_COMPACT_SYNTHESIS_PREFIX));
        assert_eq!(
            decode_compact_synthesis_encrypted_content(&encoded).as_deref(),
            Some("hello summary")
        );
        assert!(decode_compact_synthesis_encrypted_content("openai-opaque").is_none());
    }

    #[test]
    fn synthesizes_exactly_one_compaction_item() {
        let mut body = json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Condensed thread."}]
                },
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "extra notes"}]
                }
            ]
        });
        assert!(synthesize_compaction_client_response(&mut body, "m", true, true).unwrap());
        assert_eq!(body["output"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["output"][0]["type"], "compaction");
        let encrypted = body["output"][0]["encrypted_content"].as_str().unwrap();
        assert_eq!(
            decode_compact_synthesis_encrypted_content(encrypted).as_deref(),
            Some("Condensed thread.\n\nextra notes")
        );
    }

    #[test]
    fn leaves_native_compaction_untouched() {
        let mut body = json!({
            "id": "resp_1",
            "output": [{
                "type": "compaction",
                "encrypted_content": "openai-native"
            }]
        });
        assert!(!synthesize_compaction_client_response(&mut body, "m", true, true).unwrap());
        assert_eq!(body["output"][0]["encrypted_content"], "openai-native");
    }

    #[test]
    fn expands_synthetic_history_into_instructions() {
        let encrypted = encode_compact_synthesis_encrypted_content("prior summary", "m");
        let mut body = json!({
            "instructions": "system",
            "input": [
                {"type": "compaction", "encrypted_content": encrypted},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
            ],
            "tools": [{"type": "function", "name": "x"}]
        });
        assert!(prepare_compact_synthesis_provider_request(
            &mut body, true, true
        ));
        assert!(body["instructions"]
            .as_str()
            .unwrap()
            .contains("prior summary"));
        assert!(!body["instructions"]
            .as_str()
            .unwrap()
            .contains("CONTEXT CHECKPOINT COMPACTION"));
        // prior user message + Codex local-compaction summarization user message
        assert_eq!(body["input"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            body["input"][1]["content"][0]["text"].as_str(),
            Some(COMPACT_SUMMARIZATION_PROMPT)
        );
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn detects_compaction_trigger_operation() {
        let body = json!({
            "input": [{"type": "compaction_trigger"}]
        });
        assert!(responses_body_is_remote_compaction_request(
            "openai:responses",
            &body
        ));
        assert!(!responses_body_is_remote_compaction_request(
            "openai:responses",
            &json!({"input": [{"type": "compaction", "encrypted_content": "x"}]})
        ));
    }

    #[test]
    fn rewrites_sse_without_compaction_into_single_item_stream() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_x\",\"object\":\"response\",\"model\":\"m\",\"status\":\"in_progress\",\"output\":[]}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello summary\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_x\",\"status\":\"completed\",\"output\":[]}}\n\n",
        );
        let rewritten = maybe_rewrite_responses_sse_for_compact_synthesis(sse, "m", true, true)
            .expect("rewrite")
            .expect("should rewrite");
        assert!(rewritten.contains("\"type\":\"compaction\""));
        assert!(rewritten.contains(AETHER_COMPACT_SYNTHESIS_PREFIX));
        assert!(rewritten.contains("response.completed"));
    }

    #[test]
    fn strips_compaction_trigger_and_appends_summarization_user_message() {
        let mut body = json!({
            "instructions": "system",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "compaction_trigger"}
            ],
            "tools": [{"type": "function", "name": "x"}]
        });
        assert!(prepare_compact_synthesis_provider_request(&mut body, true, true));
        assert!(body.get("tools").is_none());
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("type").and_then(Value::as_str) != Some("compaction_trigger")));
        assert_eq!(body["instructions"].as_str(), Some("system"));
        assert!(!body["instructions"]
            .as_str()
            .unwrap()
            .contains("Summarize the conversation so far"));
        let last = body["input"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(
            last["content"][0]["text"].as_str(),
            Some(COMPACT_SUMMARIZATION_PROMPT)
        );
        assert!(last["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("CONTEXT CHECKPOINT COMPACTION"));
        // Idempotent: second prepare must not double-append the prompt.
        assert!(!prepare_compact_synthesis_provider_request(
            &mut body, true, true
        ));
        assert_eq!(body["input"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn native_encrypted_blob_is_not_decoded() {
        assert!(decode_compact_synthesis_encrypted_content("gAAAA-openai-native-ciphertext").is_none());
    }

    #[test]
    fn sse_compaction_detection_parses_events_not_raw_substring() {
        let decoy = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"mention type:compaction in text\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_x\",\"output\":[]}}\n\n",
        );
        let rewritten = maybe_rewrite_responses_sse_for_compact_synthesis(decoy, "m", true, true)
            .expect("rewrite")
            .expect("should rewrite despite decoy substring");
        assert!(rewritten.contains("\"type\":\"compaction\""));
        assert!(rewritten.contains(AETHER_COMPACT_SYNTHESIS_PREFIX));
    }

    #[test]
    fn prepared_compact_body_converts_to_openai_chat() {
        let mut body = json!({
            "model": "gpt-test",
            "instructions": "system",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "please compact"}]
                },
                {"type": "compaction_trigger"}
            ],
            "tools": [{"type": "function", "name": "lookup", "parameters": {"type": "object"}}]
        });
        assert!(prepare_compact_synthesis_provider_request(&mut body, true, true));
        crate::formats::registry::convert_request_pure("openai:responses", "openai:chat", &body)
            .expect("prepared compact body should convert to chat");
    }

    #[test]
    fn unprepared_compaction_trigger_still_blocks_cross_format() {
        let body = json!({
            "model": "gpt-test",
            "input": [
                {"type": "message", "role": "user", "content": "hello"},
                {"type": "compaction_trigger"}
            ]
        });
        let err = crate::formats::registry::convert_request_pure(
            "openai:responses",
            "openai:chat",
            &body,
        )
        .expect_err("raw compaction_trigger must remain lossy");
        assert!(matches!(
            err,
            crate::FormatError::LossyConversionBlocked { .. }
        ));
    }

    #[test]
    fn empty_summary_is_committed_synthesis_failure() {
        let mut body = json!({
            "id": "resp_empty",
            "output": []
        });
        let err = synthesize_compaction_client_response(&mut body, "m", true, true)
            .expect_err("empty summary must fail synthesis");
        assert!(err.to_lowercase().contains("empty"));
    }
}
