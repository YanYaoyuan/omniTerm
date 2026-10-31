use super::anthropic;
use super::chat;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use bytes::Bytes;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ResponseItem;
use futures::stream;
use http::HeaderMap;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

fn item(value: Value) -> ResponseItem {
    serde_json::from_value(value).expect("valid response item")
}

fn history() -> Vec<ResponseItem> {
    vec![
        item(json!({"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "rules"}]})),
        item(json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "check B"}]})),
        item(json!({"type": "reasoning", "summary": [], "content": [{"type": "reasoning_text", "text": "thinking"}], "encrypted_content": null})),
        item(json!({"type": "function_call", "name": "terminal_read_context", "arguments": "{\"terminal\":\"b\"}", "call_id": "call_1"})),
        item(json!({"type": "function_call", "name": "workspace_list_terminals", "arguments": "{}", "call_id": "call_2"})),
        item(json!({"type": "function_call_output", "call_id": "call_1", "output": "log"})),
        item(json!({"type": "function_call_output", "call_id": "call_2", "output": "[]"})),
    ]
}

fn request(input: Vec<ResponseItem>) -> ResponsesApiRequest {
    let tools = serde_json::value::RawValue::from_string(
        json!([
            {"type": "function", "name": "terminal_read_context", "description": "read", "strict": false, "parameters": {"type": "object", "properties": {}}},
            {"type": "custom", "name": "apply_patch", "description": "freeform", "format": {}},
            {"type": "web_search"},
        ])
        .to_string(),
    )
    .expect("valid tools");
    ResponsesApiRequest {
        model: "model-id".to_string(),
        stream: true,
        service_tier: None,
        input,
        tools: Some(Arc::<serde_json::value::RawValue>::from(tools).into()),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: false,
        reasoning: None,
        store: false,
        stream_options: None,
        include: Vec::new(),
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
        access_programs: None,
    }
}

fn response(sse: String) -> StreamResponse {
    StreamResponse {
        status: StatusCode::OK,
        headers: HeaderMap::new(),
        bytes: Box::pin(stream::iter(vec![Ok::<Bytes, TransportError>(Bytes::from(
            sse,
        ))])),
    }
}

async fn collect(mut stream: ResponseStream) -> (Vec<ResponseEvent>, Option<String>) {
    let mut events = Vec::new();
    while let Some(event) = stream.rx_event.recv().await {
        match event {
            Ok(event) => events.push(event),
            Err(error) => return (events, Some(error.to_string())),
        }
    }
    (events, None)
}

fn data(chunks: &[Value], done: bool) -> String {
    let mut sse: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    if done {
        sse.push_str("data: [DONE]\n\n");
    }
    sse
}

fn function_calls(events: &[ResponseEvent]) -> Vec<(String, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            }) => Some((call_id.clone(), name.clone(), arguments.clone())),
            _ => None,
        })
        .collect()
}

const IDLE: Duration = Duration::from_secs(5);

#[test]
fn chat_request_maps_roles_tools_and_merges_parallel_calls() {
    let body = chat::encode_request(&request(history()));
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["content"], "check B");
    assert_eq!(messages[2]["reasoning_content"], "thinking");
    assert_eq!(messages[2]["tool_calls"].as_array().map(Vec::len), Some(2));
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_1");
    assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["tools"][0]["function"]["name"], "terminal_read_context");
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn anthropic_request_lifts_system_and_groups_tool_blocks() {
    let body = anthropic::encode_request(&request(history()));
    assert_eq!(body["system"], "rules");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["type"], "tool_use");
    assert_eq!(messages[1]["content"][0]["input"], json!({"terminal": "b"}));
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"][1]["tool_use_id"], "call_2");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
}

#[tokio::test]
async fn chat_assembles_tool_arguments_across_fragments_by_index() {
    let sse = data(
        &[
            json!({"id": "c1", "choices": [{"delta": {"content": "Looking"}}]}),
            json!({"id": "c1", "choices": [{"delta": {"tool_calls": [
                {"index": 1, "id": "b", "function": {"name": "second", "arguments": "{\"x\""}},
                {"index": 0, "id": "a", "function": {"name": "first", "arguments": ""}},
            ]}}]}),
            json!({"id": "c1", "choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "{}"}},
                {"index": 1, "function": {"arguments": ":1}"}},
            ]}, "finish_reason": "tool_calls"}]}),
            json!({"id": "c1", "choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 3, "prompt_tokens_details": {"cached_tokens": 2}}}),
        ],
        /*done*/ true,
    );
    let (events, error) = collect(chat::spawn_stream(response(sse), IDLE)).await;
    assert_eq!(error, None);
    assert_eq!(
        function_calls(&events),
        vec![
            ("a".to_string(), "first".to_string(), "{}".to_string()),
            ("b".to_string(), "second".to_string(), "{\"x\":1}".to_string()),
        ]
    );
    assert!(matches!(
        events.last(),
        Some(ResponseEvent::Completed { response_id, token_usage: Some(usage), .. })
            if response_id == "c1" && usage.total_tokens == 10 && usage.cached_input_tokens == 2
    ));
}

#[tokio::test]
async fn chat_rejects_invalid_or_truncated_tool_arguments_without_emitting_calls() {
    let partial = json!({"choices": [{"delta": {"tool_calls": [
        {"index": 0, "id": "a", "function": {"name": "run", "arguments": "{\"command\":\"rm"}},
        {"index": 1, "id": "b", "function": {"name": "ok", "arguments": "{}"}},
    ]}}]});
    let finished = json!({"choices": [{"delta": {}, "finish_reason": "length"}]});
    for sse in [
        data(&[partial.clone(), finished], /*done*/ true),
        data(&[partial], /*done*/ false),
    ] {
        let (events, error) = collect(chat::spawn_stream(response(sse), IDLE)).await;
        assert!(function_calls(&events).is_empty());
        assert!(!events.iter().any(|event| matches!(event, ResponseEvent::Completed { .. })));
        assert!(error.is_some_and(|error| error.contains("nothing")));
    }
}

#[tokio::test]
async fn chat_reports_errors_inside_the_stream() {
    let sse = data(
        &[json!({"error": {"message": "slow down", "code": 429}})],
        /*done*/ false,
    );
    let (_, error) = collect(chat::spawn_stream(response(sse), IDLE)).await;
    assert!(error.is_some_and(|error| error.contains("slow down")));
}

fn anthropic_stream(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap_or("")))
        .collect()
}

#[tokio::test]
async fn anthropic_streams_text_and_tool_use_blocks() {
    let sse = anthropic_stream(&[
        json!({"type": "message_start", "message": {"id": "msg_1", "usage": {"input_tokens": 5, "cache_read_input_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hi"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "run", "input": {}}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"a\""}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": ":2}"}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 4}}),
        json!({"type": "message_stop"}),
    ]);
    let (events, error) = collect(anthropic::spawn_stream(response(sse), IDLE)).await;
    assert_eq!(error, None);
    assert!(events.iter().any(|event| matches!(event, ResponseEvent::OutputTextDelta(text) if text == "Hi")));
    assert_eq!(
        function_calls(&events),
        vec![("toolu_1".to_string(), "run".to_string(), "{\"a\":2}".to_string())]
    );
    assert!(matches!(
        events.last(),
        Some(ResponseEvent::Completed { response_id, token_usage: Some(usage), .. })
            if response_id == "msg_1" && usage.input_tokens == 6 && usage.output_tokens == 4
    ));
}

#[tokio::test]
async fn anthropic_does_not_run_a_tool_cut_off_before_message_stop() {
    let sse = anthropic_stream(&[
        json!({"type": "message_start", "message": {"id": "msg_1"}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "t", "name": "run", "input": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
    ]);
    let (events, error) = collect(anthropic::spawn_stream(response(sse), IDLE)).await;
    assert!(function_calls(&events).is_empty());
    assert!(error.is_some());
}

#[tokio::test]
async fn anthropic_reports_stream_errors() {
    let sse = anthropic_stream(&[json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})]);
    let (_, error) = collect(anthropic::spawn_stream(response(sse), IDLE)).await;
    assert!(error.is_some_and(|error| error.contains("Overloaded")));
}
