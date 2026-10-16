use std::{collections::BTreeMap, io};

use serde_json::Value;

use super::request::OpenAiResponsesRequestContractViolation;
use super::{deepseek_opaque_reasoning_item_is_replayable, openai_responses_request_operation};

// Bound the expansion independently of the inbound body limit. Large opaque states must
// remain intact; rejecting a repair is safer than truncating them or sending a partial fix.
const MAX_REPLAYED_REASONING_ITEMS: usize = 4096;
const MAX_REPLAYED_REASONING_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiResponsesToolReplayPolicy {
    #[default]
    Preserve,
    DeepSeekAdjacentPairs,
    DeepSeekGroupedOutputs,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiResponsesToolReplayNormalization {
    pub reordered_tool_outputs: usize,
    pub replayed_reasoning_items: usize,
    pub skipped_tool_batches: usize,
    pub stripped_reasoning_items: usize,
    pub summary_reasoning_items: usize,
    pub placeholder_reasoning_items: usize,
    pub reordered_reasoning_steps: usize,
}

impl OpenAiResponsesToolReplayNormalization {
    pub fn changed(self) -> bool {
        self.reordered_tool_outputs > 0
            || self.replayed_reasoning_items > 0
            || self.summary_reasoning_items > 0
            || self.placeholder_reasoning_items > 0
            || self.reordered_reasoning_steps > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ToolKind {
    Function,
    Custom,
    LocalShell,
}

type ToolKey<'a> = (ToolKind, &'a str);

fn tool_key(item: &Value) -> Option<(ToolKey<'_>, bool)> {
    let (kind, is_output) = match item.get("type")?.as_str()? {
        "function_call" => (ToolKind::Function, false),
        "function_call_output" => (ToolKind::Function, true),
        "custom_tool_call" => (ToolKind::Custom, false),
        "custom_tool_call_output" => (ToolKind::Custom, true),
        "local_shell_call" => (ToolKind::LocalShell, false),
        "local_shell_call_output" => (ToolKind::LocalShell, true),
        _ => return None,
    };
    let call_id = item.get("call_id")?.as_str()?;
    (!call_id.trim().is_empty()).then_some(((kind, call_id), is_output))
}

fn is_context_message(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        None | Some("message")
    ) && matches!(
        item.get("role").and_then(Value::as_str),
        Some("assistant" | "developer" | "system")
    )
}

fn complete_batch(items: &[Value], start: usize) -> (usize, Option<BTreeMap<usize, usize>>) {
    let mut pending = BTreeMap::new();
    let mut pairs = BTreeMap::new();
    let mut saw_output = false;
    for (index, item) in items.iter().enumerate().skip(start) {
        match tool_key(item) {
            Some((key, false)) if !saw_output => {
                if pending.insert(key, index).is_some() {
                    return (index + 1, None);
                }
            }
            Some((key, true)) => {
                let Some(call_index) = pending.remove(&key) else {
                    return (index + 1, None);
                };
                saw_output = true;
                pairs.insert(call_index, index);
                if pending.is_empty() {
                    return (index + 1, Some(pairs));
                }
            }
            None if is_context_message(item) => {}
            // A user turn, new reasoning, an unknown item, or a new call after outputs
            // started is a boundary. Never search the rest of the thread for a late result.
            _ => return (index, None),
        }
    }
    (items.len(), None)
}

fn append_grouped_segment(
    segment: &mut Vec<usize>,
    pairs: &BTreeMap<usize, usize>,
    replay: &mut Vec<usize>,
) {
    replay.extend(segment.iter().copied());
    replay.extend(segment.iter().filter_map(|index| pairs.get(index)).copied());
    segment.clear();
}

fn append_grouped_batch(
    items: &[Value],
    start: usize,
    end: usize,
    pairs: &BTreeMap<usize, usize>,
    replay: &mut Vec<usize>,
) {
    let mut segment = Vec::new();
    let mut saw_output = false;
    for (index, item) in items.iter().enumerate().take(end).skip(start) {
        match tool_key(item) {
            Some((_, false)) => segment.push(index),
            Some((_, true)) => saw_output = true,
            None if !saw_output
                && item.get("role").and_then(Value::as_str) == Some("assistant") =>
            {
                segment.push(index);
            }
            _ => {
                // A hook closes the assistant segment. Its later calls must not
                // inherit the previous segment's opaque reasoning.
                append_grouped_segment(&mut segment, pairs, replay);
                replay.push(index);
            }
        }
    }
    append_grouped_segment(&mut segment, pairs, replay);
}

#[derive(Default)]
struct SerializedSize(usize);

impl io::Write for SerializedSize {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_size(value: &Value) -> usize {
    let mut size = SerializedSize::default();
    // Count the exact JSON wire size without allocating a temporary serialized body.
    serde_json::to_writer(&mut size, value).expect("Value serialization to a counting writer");
    size.0
}

/// Normalizes complete DeepSeek tool batches using the selected replay contract.
///
/// The client history is untouched: only the provider-bound body is normalized. Item IDs are
/// local identities; tool pairing uses `call_id` and the matching call/output type instead.
/// Grouped replay keeps each uninterrupted assistant segment together with its results;
/// context messages remain boundaries. Adjacent replay retains the legacy bounded-copy behavior.
pub fn normalize_openai_responses_tool_replay(
    body: &mut Value,
    provider_api_format: &str,
    policy: OpenAiResponsesToolReplayPolicy,
) -> Result<OpenAiResponsesToolReplayNormalization, OpenAiResponsesRequestContractViolation> {
    let mut report = OpenAiResponsesToolReplayNormalization::default();
    if policy == OpenAiResponsesToolReplayPolicy::Preserve
        || crate::normalize_api_format_alias(provider_api_format) != "openai:responses"
        || openai_responses_request_operation(provider_api_format, body).is_some()
    {
        return Ok(report);
    }
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return Ok(report);
    };
    let mut counts = BTreeMap::<&str, (usize, usize)>::new();
    for item in items.iter() {
        if let Some((key, is_output)) = tool_key(item) {
            let count = counts.entry(key.1).or_default();
            if is_output {
                count.1 += 1;
            } else {
                count.0 += 1;
            }
        }
    }
    if !counts.values().any(|(calls, _)| *calls > 0) {
        return Ok(report);
    }

    let mut replay = Vec::with_capacity(items.len());
    let mut reasoning = Vec::<(usize, Option<usize>)>::new();
    let mut reasoning_copies = BTreeMap::<usize, usize>::new();
    let mut replayed_bytes = 0usize;
    let mut index = 0;
    while index < items.len() {
        let item = &items[index];
        if matches!(tool_key(item), Some((_, false))) {
            let (end, pairs) = complete_batch(items, index);
            let pairs = pairs.filter(|pairs| {
                pairs.keys().all(|call| {
                    tool_key(&items[*call])
                        .is_some_and(|(key, _)| counts.get(key.1) == Some(&(1, 1)))
                })
            });
            if let Some(pairs) = pairs {
                if policy == OpenAiResponsesToolReplayPolicy::DeepSeekGroupedOutputs {
                    // One uninterrupted parallel assistant turn owns one reasoning
                    // state. Never duplicate it or carry it across a hook boundary.
                    let start = replay.len();
                    append_grouped_batch(items, index, end, &pairs, &mut replay);
                    report.reordered_tool_outputs += replay[start..]
                        .iter()
                        .enumerate()
                        .filter(|(offset, item)| {
                            **item != index + offset
                                && matches!(tool_key(&items[**item]), Some((_, true)))
                        })
                        .count();
                    reasoning.clear();
                    index = end;
                    continue;
                }
                let mut calls_in_turn = 0;
                for (offset, item) in items[index..end].iter().enumerate() {
                    let item_index = index + offset;
                    if let Some(&output_index) = pairs.get(&item_index) {
                        // Pairing closes a replayed turn. Extra calls in the original parallel
                        // turn need that same opaque reasoning, never an earlier turn's state.
                        if calls_in_turn > 0 {
                            for (reasoning_index, size) in &mut reasoning {
                                let size = *size.get_or_insert_with(|| {
                                    serialized_size(&items[*reasoning_index])
                                });
                                if report.replayed_reasoning_items >= MAX_REPLAYED_REASONING_ITEMS {
                                    return Err(OpenAiResponsesRequestContractViolation {
                                        field: "input",
                                        reason: "DeepSeek tool replay exceeds the 4096 additional reasoning items budget",
                                    });
                                }
                                replayed_bytes = replayed_bytes.saturating_add(size);
                                if replayed_bytes > MAX_REPLAYED_REASONING_BYTES {
                                    return Err(OpenAiResponsesRequestContractViolation {
                                        field: "input",
                                        reason: "DeepSeek tool replay exceeds the 8 MiB additional reasoning bytes budget",
                                    });
                                }
                                replay.push(*reasoning_index);
                                *reasoning_copies.entry(*reasoning_index).or_default() += 1;
                                report.replayed_reasoning_items += 1;
                            }
                        }
                        calls_in_turn += 1;
                        replay.push(item_index);
                        replay.push(output_index);
                        report.reordered_tool_outputs +=
                            usize::from(output_index != item_index + 1);
                    } else if matches!(tool_key(item), Some((_, true))) {
                        reasoning.clear();
                        calls_in_turn = 0;
                    } else {
                        replay.push(item_index);
                        if item.get("role").and_then(Value::as_str) != Some("assistant") {
                            reasoning.clear();
                            calls_in_turn = 0;
                        }
                    }
                }
            } else {
                // Removing this boundary would make a later finalization see one
                // parallel batch and attach an earlier turn's state to later calls.
                // Reject the ambiguous history rather than forwarding a body whose
                // segmentation cannot be represented after reasoning filtering.
                if items.get(end).is_some_and(|item| {
                    item.get("type").and_then(Value::as_str) == Some("reasoning")
                        && !super::openai_responses_reasoning_item_is_replayable(
                            item,
                            super::OpenAiResponsesReasoningReplayPolicy::DeepSeekOpaque,
                        )
                }) {
                    return Err(OpenAiResponsesRequestContractViolation {
                        field: "input",
                        reason: "DeepSeek tool replay cannot preserve an incomplete tool batch boundary after filtering incompatible reasoning",
                    });
                }
                replay.extend(index..end);
                report.skipped_tool_batches += 1;
            }
            reasoning.clear();
            index = end;
            continue;
        }

        if item.get("type").and_then(Value::as_str) == Some("reasoning") {
            if index == 0
                || items[index - 1].get("type").and_then(Value::as_str) != Some("reasoning")
            {
                reasoning.clear();
            }
            if item
                .as_object()
                .is_some_and(deepseek_opaque_reasoning_item_is_replayable)
                && super::openai_responses_reasoning_item_is_replayable(
                    item,
                    super::OpenAiResponsesReasoningReplayPolicy::DeepSeekOpaque,
                )
            {
                reasoning.push((index, None));
            } else {
                reasoning.clear();
            }
        } else if !is_context_message(item)
            || item.get("role").and_then(Value::as_str) != Some("assistant")
        {
            reasoning.clear();
        }
        replay.push(index);
        index += 1;
    }
    if !report.changed() {
        return Ok(report);
    }
    let mut original = std::mem::take(items)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    *items = replay
        .into_iter()
        .map(|index| {
            // All budgets have passed. Retain the original state until its last use so
            // each additional occurrence makes exactly one clone, with no extra pool.
            if let Some(copies) = reasoning_copies
                .get_mut(&index)
                .filter(|copies| **copies > 0)
            {
                *copies -= 1;
                original[index]
                    .as_ref()
                    .expect("reasoning retained until last use")
                    .clone()
            } else {
                original[index]
                    .take()
                    .expect("unique original replay index")
            }
        })
        .collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn normalize(body: &mut Value) -> OpenAiResponsesToolReplayNormalization {
        normalize_openai_responses_tool_replay(
            body,
            "openai:responses",
            OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
        )
        .expect("replay within budget")
    }

    fn reasoning() -> Value {
        json!({
            "type": "reasoning",
            "encrypted_content": "opaque-provider-state",
            "content": [{"type": "reasoning_text", "text": "original thinking"}],
            "future_reasoning_field": {"preserve": true}
        })
    }

    fn parallel_batch(reasoning_items: usize, calls: usize, state_bytes: usize) -> Value {
        let mut items = Vec::new();
        for _ in 0..reasoning_items {
            let mut item = reasoning();
            item["encrypted_content"] = json!("x".repeat(state_bytes));
            items.push(item);
        }
        for index in 0..calls {
            items.push(json!({"type": "function_call", "call_id": format!("call_{index}")}));
        }
        for index in 0..calls {
            items.push(json!({"type": "function_call_output", "call_id": format!("call_{index}"), "output": "ok"}));
        }
        json!({"input": items})
    }

    #[test]
    fn tool_replay_rejects_removed_batch_boundaries_before_applying_earlier_repairs() {
        let mut body = parallel_batch(1, 2, 1);
        body["input"].as_array_mut().unwrap().extend([
            json!({"type": "function_call", "call_id": "incomplete"}),
            json!({"type": "reasoning", "id": "item_other", "summary": []}),
            json!({"type": "function_call_output", "call_id": "incomplete", "output": "late"}),
        ]);
        let original = body.clone();
        let error = normalize_openai_responses_tool_replay(
            &mut body,
            "openai:responses",
            OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
        )
        .unwrap_err();
        assert!(error.reason.contains("batch boundary"));
        assert_eq!(body, original);
    }

    #[test]
    fn tool_replay_rejects_reasoning_byte_expansion_without_mutating_input() {
        let mut body = parallel_batch(1, 64, 1024 * 1024);
        let original = body.clone();
        let error = normalize_openai_responses_tool_replay(
            &mut body,
            "openai:responses",
            OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
        )
        .unwrap_err();
        assert_eq!(error.field, "input");
        assert!(error.reason.contains("8 MiB"));
        assert_eq!(body, original);
    }

    #[test]
    fn tool_replay_limits_reasoning_count_including_copies_from_earlier_batches() {
        let mut body = parallel_batch(64, 66, 1);
        let original = body.clone();
        let error = normalize_openai_responses_tool_replay(
            &mut body,
            "openai:responses",
            OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
        )
        .unwrap_err();
        assert!(error.reason.contains("4096"));
        assert_eq!(body, original);

        // Individually below the byte budget, but above it in aggregate. The first
        // completed batch must remain untouched when a later batch exceeds the budget.
        let mut body = parallel_batch(1, 5, 1024 * 1024);
        let mut next = parallel_batch(1, 5, 1024 * 1024);
        for item in next["input"].as_array_mut().unwrap() {
            if let Some(id) = item.get("call_id").and_then(Value::as_str) {
                item["call_id"] = json!(format!("second_{id}"));
            }
        }
        body["input"]
            .as_array_mut()
            .unwrap()
            .extend(next["input"].as_array().unwrap().iter().cloned());
        let original = body.clone();
        let error = normalize_openai_responses_tool_replay(
            &mut body,
            "openai:responses",
            OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
        )
        .unwrap_err();
        assert!(error.reason.contains("8 MiB"));
        assert_eq!(body, original);
    }

    #[test]
    fn tool_replay_accepts_exact_budgets_and_does_not_charge_original_reasoning() {
        let mut body = parallel_batch(1, 2, 0);
        let empty_size = serialized_size(&body["input"][0]);
        body["input"][0]["encrypted_content"] =
            json!("x".repeat(MAX_REPLAYED_REASONING_BYTES - empty_size));
        assert_eq!(
            serialized_size(&body["input"][0]),
            MAX_REPLAYED_REASONING_BYTES
        );
        assert_eq!(normalize(&mut body).replayed_reasoning_items, 1);
        let once = body.clone();
        assert!(!normalize(&mut body).changed());
        assert_eq!(body, once);

        let mut body = parallel_batch(64, 65, 1);
        assert_eq!(
            normalize(&mut body).replayed_reasoning_items,
            MAX_REPLAYED_REASONING_ITEMS
        );
        assert!(!normalize(&mut body).changed());
        let mut body = parallel_batch(1, 1, MAX_REPLAYED_REASONING_BYTES + 1);
        assert!(!normalize(&mut body).changed());
    }

    #[test]
    fn repairs_openviking_reports_interleaved_in_parallel_outputs_losslessly() {
        let mut body = json!({
            "parallel_tool_calls": true,
            "future_request_field": {"preserve": true},
            "input": [
                {"type": "function_call", "call_id": "search", "name": "search"},
                {"type": "function_call", "call_id": "find", "name": "find", "namespace": "mcp__openviking_memory"},
                {"type": "function_call", "call_id": "exec", "name": "exec_command"},
                {"type": "message", "role": "developer", "content": "<openviking-usage>report 1</openviking-usage>"},
                {"type": "function_call_output", "call_id": "search", "output": "search results"},
                {"type": "message", "role": "developer", "content": "<openviking-usage>report 2</openviking-usage>"},
                {"type": "function_call_output", "call_id": "find", "output": "find results"},
                {"type": "function_call_output", "call_id": "exec", "output": "command results"}
            ]
        });
        let original = body["input"].as_array().unwrap().clone();
        let report = normalize(&mut body);
        let expected = [0, 4, 1, 6, 2, 7, 3, 5].map(|index| original[index].clone());
        assert_eq!(body["input"], json!(expected));
        assert_eq!(report.reordered_tool_outputs, 3);
        assert_eq!(report.replayed_reasoning_items, 0);
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["future_request_field"], json!({"preserve": true}));
        let repaired = body.clone();
        assert_eq!(
            normalize(&mut body),
            OpenAiResponsesToolReplayNormalization::default()
        );
        assert_eq!(body, repaired);
    }

    #[test]
    fn replays_only_same_turn_reasoning_for_parallel_calls_after_an_assistant_preamble() {
        let state = reasoning();
        let mut body = json!({"input": [
            state,
            {"type": "message", "role": "assistant", "content": "I will run both tools"},
            {"type": "function_call", "call_id": "a"},
            {"type": "function_call", "call_id": "b"},
            {"type": "function_call_output", "call_id": "b", "output": "second finished first"},
            {"type": "function_call_output", "call_id": "a", "output": "first"},
            {"type": "function_call", "call_id": "c"},
            {"type": "function_call", "call_id": "d"},
            {"type": "function_call_output", "call_id": "c", "output": "third"},
            {"type": "function_call_output", "call_id": "d", "output": "fourth"}
        ]});
        let original = body["input"].as_array().unwrap().clone();
        let report = normalize(&mut body);
        assert_eq!(
            body["input"],
            json!([
                original[0],
                original[1],
                original[2],
                original[5],
                state,
                original[3],
                original[4],
                original[6],
                original[8],
                original[7],
                original[9]
            ])
        );
        assert_eq!(report.replayed_reasoning_items, 1);
        let repaired = body.clone();
        assert_eq!(
            normalize(&mut body),
            OpenAiResponsesToolReplayNormalization::default()
        );
        assert_eq!(body, repaired);
    }

    #[test]
    fn keeps_context_order_and_never_copies_reasoning_across_a_hook_message() {
        let mut body = json!({"input": [
            reasoning(),
            {"type": "function_call", "call_id": "a"},
            {"role": "developer", "content": "hook 1"},
            {"type": "function_call", "call_id": "b"},
            {"role": "assistant", "content": "preamble"},
            {"type": "function_call", "call_id": "c"},
            {"role": "system", "content": "hook 2"},
            {"type": "function_call_output", "call_id": "a", "output": "a"},
            {"type": "function_call_output", "call_id": "b", "output": "b"},
            {"type": "function_call_output", "call_id": "c", "output": "c"}
        ]});
        let original = body["input"].as_array().unwrap().clone();
        let report = normalize(&mut body);
        let expected = [0, 1, 7, 2, 3, 8, 4, 5, 9, 6].map(|index| original[index].clone());
        assert_eq!(body["input"], json!(expected));
        assert_eq!(report.replayed_reasoning_items, 0);
    }

    #[test]
    fn pairs_function_custom_and_shell_tools_on_call_id_instead_of_item_id() {
        for (call_type, output_type) in [
            ("function_call", "function_call_output"),
            ("custom_tool_call", "custom_tool_call_output"),
            ("local_shell_call", "local_shell_call_output"),
        ] {
            let mut body = json!({"input": [
                {"type": call_type, "id": "local-uuid", "call_id": "api-call-id", "future_call_field": 7},
                {"type": "message", "role": "developer", "content": "hook context"},
                {"type": output_type, "id": "output-uuid", "call_id": "api-call-id", "output": [{"type": "input_text", "text": "ok"}]}
            ]});
            let original = body["input"].as_array().unwrap().clone();
            assert_eq!(normalize(&mut body).reordered_tool_outputs, 1);
            assert_eq!(
                body["input"],
                json!([original[0], original[2], original[1]])
            );
        }
    }

    #[test]
    fn leaves_incomplete_ambiguous_or_wrong_type_batches_unchanged() {
        for input in [
            json!([
                {"type": "function_call", "call_id": "missing"},
                {"type": "function_call", "call_id": "found"},
                {"role": "developer", "content": "context"},
                {"type": "function_call_output", "call_id": "found", "output": "ok"}
            ]),
            json!([
                {"type": "function_call", "call_id": "duplicate"},
                {"role": "developer", "content": "context"},
                {"type": "function_call_output", "call_id": "duplicate", "output": "one"},
                {"type": "function_call_output", "call_id": "duplicate", "output": "two"}
            ]),
            json!([
                {"type": "function_call", "call_id": "duplicate"},
                {"type": "function_call", "call_id": "duplicate"},
                {"role": "developer", "content": "context"},
                {"type": "function_call_output", "call_id": "duplicate", "output": "ok"}
            ]),
            json!([
                {"type": "function_call", "id": "local-id"},
                {"role": "developer", "content": "context"},
                {"type": "function_call_output", "call_id": "local-id", "output": "ok"}
            ]),
            json!([
                {"type": "function_call", "call_id": "wrong-type"},
                {"role": "developer", "content": "context"},
                {"type": "custom_tool_call_output", "call_id": "wrong-type", "output": "ok"}
            ]),
            json!([
                {"type": "function_call", "call_id": "shared"},
                {"role": "developer", "content": "context"},
                {"type": "function_call_output", "call_id": "shared", "output": "function result"},
                {"type": "custom_tool_call", "call_id": "shared"},
                {"role": "developer", "content": "context"},
                {"type": "custom_tool_call_output", "call_id": "shared", "output": "custom result"}
            ]),
        ] {
            let mut body = json!({"input": input});
            let original = body.clone();
            assert!(!normalize(&mut body).changed());
            assert_eq!(body, original);
        }
    }

    #[test]
    fn never_pulls_results_across_user_reasoning_unknown_items_or_an_incomplete_turn() {
        for boundary in [
            json!({"type": "message", "role": "user", "content": "another turn"}),
            reasoning(),
            json!({"type": "future_provider_item", "opaque": true}),
        ] {
            let mut body = json!({"input": [
                {"type": "function_call", "call_id": "old"},
                boundary,
                {"type": "function_call_output", "call_id": "old", "output": "late result"}
            ]});
            let original = body.clone();
            assert_eq!(normalize(&mut body).skipped_tool_batches, 1);
            assert_eq!(body, original);
        }
        let mut body = json!({"input": [
            {"type": "function_call", "call_id": "a"},
            {"type": "function_call", "call_id": "b"},
            {"type": "function_call_output", "call_id": "a", "output": "first"},
            {"type": "function_call", "call_id": "c"},
            {"role": "developer", "content": "hook"},
            {"type": "function_call_output", "call_id": "c", "output": "new batch"},
            {"type": "function_call_output", "call_id": "b", "output": "late old result"}
        ]});
        let original = body["input"].as_array().unwrap().clone();
        let report = normalize(&mut body);
        assert_eq!(report.skipped_tool_batches, 1);
        assert_eq!(
            body["input"],
            json!([
                original[0],
                original[1],
                original[2],
                original[3],
                original[5],
                original[4],
                original[6]
            ])
        );
    }

    #[test]
    fn leaves_other_policies_formats_compaction_and_output_only_continuations_unchanged() {
        let input = json!([
            {"type": "function_call", "call_id": "a"},
            {"role": "developer", "content": "context"},
            {"type": "function_call_output", "call_id": "a", "output": "ok"}
        ]);
        for (format, policy, input) in [
            (
                "openai:responses",
                OpenAiResponsesToolReplayPolicy::Preserve,
                input.clone(),
            ),
            (
                "openai:responses:compact",
                OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
                input.clone(),
            ),
            (
                "openai:chat",
                OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
                input.clone(),
            ),
            (
                "openai:responses",
                OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
                json!([
                    input[0], input[1], input[2], {"type": "compaction_trigger"}
                ]),
            ),
            (
                "openai:responses",
                OpenAiResponsesToolReplayPolicy::DeepSeekAdjacentPairs,
                json!([input[1], input[2]]),
            ),
        ] {
            let mut body = json!({"previous_response_id": "resp_stored", "input": input});
            let original = body.clone();
            assert_eq!(
                normalize_openai_responses_tool_replay(&mut body, format, policy).unwrap(),
                OpenAiResponsesToolReplayNormalization::default()
            );
            assert_eq!(body, original);
        }
    }

    #[test]
    fn malformed_and_non_array_inputs_are_noops() {
        for mut body in [
            Value::Null,
            json!({}),
            json!({"input": "hello"}),
            json!({"input": [null, 7, {"type": "function_call", "call_id": ""}]}),
        ] {
            let original = body.clone();
            assert!(!normalize(&mut body).changed());
            assert_eq!(body, original);
        }
    }
}
