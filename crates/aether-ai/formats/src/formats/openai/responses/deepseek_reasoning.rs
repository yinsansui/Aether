use serde_json::{json, Value};

use super::{
    deepseek_opaque_reasoning_item_is_replayable, request::OpenAiResponsesRequestContractViolation,
    OpenAiResponsesToolReplayNormalization, GEMINI_TOOL_SIGNATURE_CARRIER_PREFIX,
};

const MAX_ADDED_BYTES: usize = 8 * 1024 * 1024;
const MAX_ADDED_ITEMS: usize = 4096;

pub(crate) fn deepseek_responses_thinking_enabled(body: &Value) -> bool {
    !body["thinking"]["type"]
        .as_str()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("disabled"))
        && !body
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .or_else(|| body["reasoning"]["effort"].as_str())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("none"))
}

fn is_reasoning(item: &Value) -> bool {
    item["type"] == "reasoning"
}

fn has_text(item: &Value) -> bool {
    item["content"].as_array().is_some_and(|parts| {
        parts.iter().any(|part| {
            part["type"] == "reasoning_text"
                && part["text"].as_str().is_some_and(|text| !text.is_empty())
        })
    })
}

fn summary_parts(item: &Value) -> impl Iterator<Item = &str> {
    item["summary"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "summary_text")
        .filter_map(|part| part["text"].as_str())
        .filter(|text| !text.is_empty())
}

fn is_signature_carrier(item: &Value) -> bool {
    item["encrypted_content"]
        .as_str()
        .is_some_and(|value| value.starts_with(GEMINI_TOOL_SIGNATURE_CARRIER_PREFIX))
}

fn needs_text_compat(item: &Value) -> bool {
    is_reasoning(item)
        && !is_signature_carrier(item)
        && (item["id"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id.starts_with("rs"))
            || has_text(item)
            || summary_parts(item).next().is_some())
}

/// Text is a compatibility fallback, never a replacement for available native state.
/// Plan the expansion before allocating strings or modifying provider-owned items.
pub(crate) fn normalize_deepseek_reasoning_text(
    body: &mut Value,
    report: &mut OpenAiResponsesToolReplayNormalization,
) -> Result<(), OpenAiResponsesRequestContractViolation> {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let mut added_bytes = 0usize;
    for item in items.iter().filter(|item| needs_text_compat(item)) {
        if !has_text(item) {
            let mut length = 0usize;
            let mut count = 0usize;
            for text in summary_parts(item) {
                length = length.saturating_add(text.len());
                count += 1;
            }
            added_bytes =
                added_bytes.saturating_add(length.saturating_add(count.saturating_sub(1)).max(1));
            if added_bytes > MAX_ADDED_BYTES {
                return Err(OpenAiResponsesRequestContractViolation {
                    field: "input",
                    reason: "DeepSeek reasoning repair exceeds the 8 MiB additional text budget",
                });
            }
        }
    }
    for item in items.iter_mut().filter(|item| needs_text_compat(item)) {
        let native = item
            .as_object()
            .is_some_and(deepseek_opaque_reasoning_item_is_replayable);
        let text_present = has_text(item);
        if !text_present {
            let mut text = String::new();
            for part in summary_parts(item) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(part);
            }
            if text.is_empty() {
                text.push(' ');
                report.placeholder_reasoning_items += 1;
            } else {
                report.summary_reasoning_items += 1;
            }
            if native {
                let part = item["content"]
                    .as_array_mut()
                    .expect("native reasoning content")
                    .iter_mut()
                    .find(|part| part["type"] == "reasoning_text" && part["text"].is_string())
                    .expect("native reasoning text part");
                part["text"] = Value::String(text);
            } else {
                item["content"] = json!([{"type": "reasoning_text", "text": text}]);
            }
        }
        // Foreign IDs/ciphertext do not become DeepSeek state by gaining text.
        if !native {
            let object = item.as_object_mut().expect("reasoning item object");
            object.remove("id");
            object.remove("encrypted_content");
        }
    }
    Ok(())
}

fn is_call(item: &Value) -> bool {
    matches!(
        item["type"].as_str(),
        Some("function_call" | "custom_tool_call" | "local_shell_call")
    )
}

fn is_output(item: &Value) -> bool {
    matches!(
        item["type"].as_str(),
        Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output")
    )
}

fn is_assistant_step(item: &Value) -> bool {
    is_reasoning(item)
        || is_call(item)
        || (matches!(item["type"].as_str(), None | Some("message")) && item["role"] == "assistant")
}

/// Move items by index; never clone a whole request, a reasoning state, or a tool output.
pub(crate) fn normalize_deepseek_reasoning_steps(
    body: &mut Value,
    report: &mut OpenAiResponsesToolReplayNormalization,
) -> Result<(), OpenAiResponsesRequestContractViolation> {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let mut plan = Vec::with_capacity(items.len());
    let mut index = 0;
    let mut after_output = false;
    let mut added = 0;
    let mut reordered = 0;
    while index < items.len() {
        if !is_assistant_step(&items[index]) {
            // Unknown items and context messages are boundaries, not permission to
            // borrow the previous step's reasoning.
            after_output = is_output(&items[index]);
            plan.push(Some(index));
            index += 1;
            continue;
        }
        let start = index;
        let mut first_reasoning = None;
        let mut calls = false;
        while index < items.len() && is_assistant_step(&items[index]) {
            // Reasoning after a call starts a new step, including when the
            // previous call's output has not arrived yet.
            if calls && is_reasoning(&items[index]) {
                break;
            }
            if is_reasoning(&items[index]) {
                let rank = |item: &Value| {
                    if item
                        .as_object()
                        .is_some_and(deepseek_opaque_reasoning_item_is_replayable)
                    {
                        2
                    } else if item["content"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part["type"] == "reasoning_text"
                                && part["text"]
                                    .as_str()
                                    .is_some_and(|text| !text.trim().is_empty())
                        })
                    }) {
                        1
                    } else {
                        0
                    }
                };
                if first_reasoning.is_none_or(|first| rank(&items[index]) > rank(&items[first])) {
                    first_reasoning = Some(index);
                }
            }
            calls |= is_call(&items[index]);
            index += 1;
        }
        if (calls || after_output) && first_reasoning != Some(start) {
            plan.push(first_reasoning);
            if first_reasoning.is_none() {
                added += 1;
                if added > MAX_ADDED_ITEMS {
                    return Err(OpenAiResponsesRequestContractViolation {
                        field: "input",
                        reason:
                            "DeepSeek reasoning repair exceeds the 4096 additional items budget",
                    });
                }
            } else {
                reordered += 1;
            }
            plan.extend(
                (start..index)
                    .filter(|item| Some(*item) != first_reasoning)
                    .map(Some),
            );
        } else {
            plan.extend((start..index).map(Some));
        }
        after_output = false;
    }
    if added == 0 && reordered == 0 {
        return Ok(());
    }
    let mut original: Vec<_> = std::mem::take(items).into_iter().map(Some).collect();
    *items = plan.into_iter().map(|index| match index {
        Some(index) => original[index].take().expect("each item moved once"),
        None => json!({"type": "reasoning", "summary": [], "content": [{"type": "reasoning_text", "text": " "}]}),
    }).collect();
    report.placeholder_reasoning_items += added;
    report.reordered_reasoning_steps += reordered;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        finalize_openai_provider_request_with_compatibility_report,
        OpenAiProviderRequestFinalization, OpenAiResponsesReasoningReplayPolicy,
    };

    fn finalize(body: &mut Value) -> OpenAiResponsesToolReplayNormalization {
        finalize_openai_provider_request_with_compatibility_report(
            body,
            OpenAiProviderRequestFinalization {
                source_api_format: "openai:responses",
                provider_api_format: "openai:responses",
                provider_type: "custom",
                provider_model: "deepseek-flash",
                source_model: "deepseek-flash",
                body_rules: None,
                upstream_is_stream: true,
                require_body_stream_field: false,
            },
            None,
            OpenAiResponsesReasoningReplayPolicy::DeepSeekOpaque,
        )
        .expect("DeepSeek request finalization")
    }

    #[test]
    fn deepseek_production_shaped_summary_history_preserves_all_144_items() {
        let native = json!({"type": "reasoning", "encrypted_content": "native-state", "content": [{"type": "reasoning_text", "text": "native text"}], "future_field": true});
        let mut input = vec![
            json!({"role": "developer", "content": "instructions"}),
            json!({"role": "developer", "content": "context"}),
            json!({"role": "user", "content": "first"}),
            json!({"role": "user", "content": "continue"}),
        ];
        for step in 0..46 {
            input.push(if step == 0 { native.clone() } else {
                json!({"type": "reasoning", "content": null, "encrypted_content": null, "summary": [{"type": "summary_text", "text": format!("summary {step}")} ]})
            });
            let count = if step == 1 { 2 } else { 1 };
            for call in 0..count {
                input.push(json!({"type": "function_call", "call_id": format!("call_{step}_{call}"), "name": "lookup", "arguments": "{}"}));
            }
            for call in 0..count {
                input.push(json!({"type": "function_call_output", "call_id": format!("call_{step}_{call}"), "output": "ok"}));
            }
        }
        assert_eq!(input.len(), 144);
        let mut body =
            json!({"model": "deepseek-flash", "reasoning": {"effort": "high"}, "input": input});
        let report = finalize(&mut body);
        assert_eq!(report.summary_reasoning_items, 45);
        assert_eq!(report.placeholder_reasoning_items, 0);
        assert_eq!(report.replayed_reasoning_items, 0);
        assert_eq!(report.stripped_reasoning_items, 0);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items.len(), 144);
        assert_eq!(items[4], native);
        assert_eq!(items.iter().filter(|item| is_reasoning(item)).count(), 46);
        assert_eq!(items.iter().filter(|item| is_call(item)).count(), 47);
        assert_eq!(items.iter().filter(|item| is_output(item)).count(), 47);
        assert!(items.iter().filter(|item| is_reasoning(item)).all(has_text));
        let once = body.clone();
        assert_eq!(
            finalize(&mut body),
            OpenAiResponsesToolReplayNormalization::default()
        );
        assert_eq!(body, once);
    }

    #[test]
    fn deepseek_reasoning_leads_notes_calls_and_post_tool_replies() {
        let mut body = json!({"input": [
            {"role": "user", "content": "lookup"},
            {"role": "assistant", "content": "let me check"},
            {"type": "reasoning", "id": "rs_aether_summary", "summary": [
                {"type": "summary_text", "text": "first"}, {"type": "summary_text", "text": "second"}
            ]},
            {"type": "function_call", "call_id": "a", "name": "lookup", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "a", "output": "ok"},
            {"role": "assistant", "content": "answer"}
        ]});
        let report = finalize(&mut body);
        assert_eq!(report.summary_reasoning_items, 1);
        assert_eq!(report.placeholder_reasoning_items, 1);
        assert_eq!(report.reordered_reasoning_steps, 1);
        assert_eq!(body["input"][1]["content"][0]["text"], "first\nsecond");
        assert!(body["input"][1].get("id").is_none());
        assert_eq!(body["input"][2]["content"], "let me check");
        assert_eq!(body["input"][5]["content"][0]["text"], " ");
        assert_eq!(body["input"][6]["content"], "answer");
    }

    #[test]
    fn deepseek_reasoning_after_calls_keeps_its_original_step_and_is_idempotent() {
        for earlier in [
            json!({"type": "reasoning", "content": [{"type": "reasoning_text", "text": "earlier text"}]}),
            json!({"type": "reasoning", "content": null, "encrypted_content": null, "summary": [{"type": "summary_text", "text": "earlier summary"}]}),
        ] {
            let native = json!({"type": "reasoning", "encrypted_content": "state-for-b", "content": [{"type": "reasoning_text", "text": "thinking for b"}]});
            let mut body = json!({"input": [
                earlier,
                {"type": "function_call", "call_id": "a", "name": "lookup", "arguments": "{}"},
                native,
                {"type": "function_call", "call_id": "b", "name": "lookup", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "b", "output": "b"},
                {"type": "function_call_output", "call_id": "a", "output": "a"}
            ]});
            finalize(&mut body);
            assert_eq!(body["input"].as_array().unwrap().len(), 6);
            assert_eq!(body["input"][1]["call_id"], "a");
            assert_eq!(body["input"][2], native);
            assert_eq!(body["input"][3]["call_id"], "b");
            assert_eq!(body["input"][4]["call_id"], "b");
            assert_eq!(body["input"][5]["call_id"], "a");
            let once = body.clone();
            assert!(!finalize(&mut body).changed());
            assert_eq!(body, once);
        }
    }

    #[test]
    fn deepseek_parallel_replay_moves_results_without_cloning_reasoning() {
        let mut body = json!({"input": [
            {"type": "reasoning", "encrypted_content": "x".repeat(1024 * 1024), "content": [{"type": "reasoning_text", "text": "native"}]},
            {"type": "function_call", "call_id": "a", "name": "lookup", "arguments": "{}"},
            {"type": "custom_tool_call", "call_id": "b", "name": "lookup", "input": "{}"},
            {"role": "developer", "content": "hook"},
            {"type": "custom_tool_call_output", "call_id": "b", "output": "b"},
            {"type": "function_call_output", "call_id": "a", "output": "a"}
        ]});
        let pointer = body["input"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .as_ptr();
        let report = finalize(&mut body);
        assert_eq!(report.replayed_reasoning_items, 0);
        assert_eq!(body["input"].as_array().unwrap().len(), 6);
        assert_eq!(
            body["input"][0]["encrypted_content"]
                .as_str()
                .unwrap()
                .as_ptr(),
            pointer
        );
        assert_eq!(body["input"][3]["call_id"], "a");
        assert_eq!(body["input"][4]["call_id"], "b");
        assert_eq!(body["input"][5]["role"], "developer");
    }

    #[test]
    fn deepseek_grouped_replay_keeps_notes_and_does_not_borrow_state_across_hooks() {
        let native = json!({"type": "reasoning", "encrypted_content": "native-for-a", "content": [{"type": "reasoning_text", "text": "native"}]});
        let mut body = json!({"input": [
            native,
            {"type": "function_call", "call_id": "a", "name": "lookup", "arguments": "{}"},
            {"role": "developer", "content": "hook"},
            {"type": "function_call", "call_id": "b", "name": "lookup", "arguments": "{}"},
            {"role": "assistant", "content": "note between parallel calls"},
            {"type": "function_call", "call_id": "c", "name": "lookup", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c", "output": "c"},
            {"type": "function_call_output", "call_id": "a", "output": "a"},
            {"type": "function_call_output", "call_id": "b", "output": "b"}
        ]});
        let report = finalize(&mut body);
        assert_eq!(report.replayed_reasoning_items, 0);
        assert_eq!(report.placeholder_reasoning_items, 1);
        assert_eq!(body["input"][0], native);
        assert_eq!(body["input"][1]["call_id"], "a");
        assert_eq!(body["input"][2]["call_id"], "a");
        assert_eq!(body["input"][3]["role"], "developer");
        assert_eq!(body["input"][4]["content"][0]["text"], " ");
        assert_eq!(body["input"][5]["call_id"], "b");
        assert_eq!(body["input"][6]["content"], "note between parallel calls");
        assert_eq!(body["input"][7]["call_id"], "c");
        assert_eq!(body["input"][8]["call_id"], "b");
        assert_eq!(body["input"][9]["call_id"], "c");
    }

    #[test]
    fn deepseek_explicit_thinking_disable_never_injects_or_converts_reasoning() {
        for control in [
            json!({"thinking": {"type": "disabled"}}),
            json!({"reasoning": {"effort": "none"}}),
            json!({"reasoning_effort": "none"}),
        ] {
            let mut body = control;
            body["input"] = json!([
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "summary"}]},
                {"type": "function_call", "call_id": "a", "name": "lookup", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "a", "output": "ok"},
                {"role": "assistant", "content": "answer"}
            ]);
            let report = finalize(&mut body);
            assert_eq!(report.summary_reasoning_items, 0);
            assert_eq!(report.placeholder_reasoning_items, 0);
            assert!(!body["input"].as_array().unwrap().iter().any(is_reasoning));
        }
    }

    #[test]
    fn deepseek_empty_native_text_keeps_opaque_state_and_unknown_parts() {
        let mut body = json!({"input": [{
            "type": "reasoning", "encrypted_content": "native-state",
            "content": [
                {"type": "reasoning_text", "text": "", "future_part_field": true},
                {"type": "future_part", "state": "keep"}
            ],
            "summary": [{"type": "summary_text", "text": "fallback summary"}]
        }]});
        let report = finalize(&mut body);
        assert_eq!(report.summary_reasoning_items, 1);
        assert_eq!(body["input"][0]["encrypted_content"], "native-state");
        assert_eq!(body["input"][0]["content"][0]["text"], "fallback summary");
        assert_eq!(body["input"][0]["content"][0]["future_part_field"], true);
        assert_eq!(body["input"][0]["content"][1]["state"], "keep");
    }

    #[test]
    fn deepseek_foreign_ciphertext_is_not_relabelled_as_native_state() {
        let mut body = json!({"input": [
            {"type": "reasoning", "id": "rs_foreign", "encrypted_content": "foreign ciphertext", "summary": [{"type": "summary_text", "text": "fallback"}]},
            {"type": "reasoning", "encrypted_content": "cpa-gemini-responses-carrier-v1:opaque", "content": [{"type": "reasoning_text", "text": "foreign signature"}]}
        ]});
        let report = finalize(&mut body);
        assert_eq!(report.stripped_reasoning_items, 1);
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert!(body["input"][0].get("id").is_none());
        assert!(body["input"][0].get("encrypted_content").is_none());
        assert_eq!(body["input"][0]["content"][0]["text"], "fallback");
    }

    #[test]
    fn deepseek_text_and_placeholder_expansion_have_transactional_limits() {
        let mut body = json!({"input": [{"type": "reasoning", "summary": [{"type": "summary_text", "text": "x".repeat(MAX_ADDED_BYTES + 1)}]}]});
        let mut report = OpenAiResponsesToolReplayNormalization::default();
        assert!(normalize_deepseek_reasoning_text(&mut body, &mut report)
            .unwrap_err()
            .reason
            .contains("8 MiB"));
        assert!(body["input"][0].get("content").is_none());
        let mut input = Vec::new();
        for index in 0..=MAX_ADDED_ITEMS {
            input.push(json!({"role": "user", "content": "boundary"}));
            input.push(json!({"type": "function_call", "call_id": format!("call_{index}")}));
        }
        let mut body = json!({"input": input});
        let original_len = body["input"].as_array().unwrap().len();
        assert!(normalize_deepseek_reasoning_steps(&mut body, &mut report)
            .unwrap_err()
            .reason
            .contains("4096"));
        assert_eq!(body["input"].as_array().unwrap().len(), original_len);
        assert!(!body["input"].as_array().unwrap().iter().any(is_reasoning));
    }
}
