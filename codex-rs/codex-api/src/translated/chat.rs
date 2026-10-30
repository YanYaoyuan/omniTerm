//! OpenAI-compatible Chat Completions, used by Qwen, Kimi, OpenRouter and custom endpoints.

use super::EventEmitter;
use super::MessagePart;
use super::ToolCallBuffer;
use super::function_tools;
use super::message_parts;
use super::output_text;
use super::reasoning_text;
use super::sse::CLOSED_EARLY;
use super::sse::Next;
use super::sse::next_event;
use super::stream_error;
use super::usage;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::error::ApiError;
use codex_client::StreamResponse;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Event;
use eventsource_stream::Eventsource;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub(crate) fn encode_request(request: &ResponsesApiRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    // Thinking models that report `reasoning_content` need it back on the assistant turn
    // that continues the same tool loop.
    let mut pending_reasoning: Option<String> = None;
    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let parts = message_parts(content);
                if parts.is_empty() {
                    continue;
                }
                let role = if matches!(role.as_str(), "developer" | "system") {
                    "system"
                } else {
                    role.as_str()
                };
                let mut message = json!({ "role": role, "content": encode_parts(&parts) });
                if role == "assistant"
                    && let Some(reasoning) = pending_reasoning.take()
                {
                    message["reasoning_content"] = json!(reasoning);
                }
                messages.push(message);
            }
            ResponseItem::Reasoning { .. } => {
                pending_reasoning = reasoning_text(item).or(pending_reasoning);
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                let call = json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments },
                });
                // Parallel calls of one model turn share a single assistant message.
                match messages.last_mut() {
                    Some(message) if message["role"] == "assistant" => {
                        if let Some(calls) = message["tool_calls"].as_array_mut() {
                            calls.push(call);
                        } else {
                            message["tool_calls"] = json!([call]);
                        }
                    }
                    _ => {
                        let mut message =
                            json!({ "role": "assistant", "content": null, "tool_calls": [call] });
                        if let Some(reasoning) = pending_reasoning.take() {
                            message["reasoning_content"] = json!(reasoning);
                        }
                        messages.push(message);
                    }
                }
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": output_text(output),
            })),
            _ => {}
        }
    }
    let tools: Vec<Value> = function_tools(request)
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                },
            })
        })
        .collect();
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(request.parallel_tool_calls);
    }
    body
}

fn encode_parts(parts: &[MessagePart]) -> Value {
    if let [MessagePart::Text(text)] = parts {
        return json!(text);
    }
    Value::Array(
        parts
            .iter()
            .map(|part| match part {
                MessagePart::Text(text) => json!({ "type": "text", "text": text }),
                MessagePart::ImageUrl(url) => {
                    json!({ "type": "image_url", "image_url": { "url": url } })
                }
            })
            .collect(),
    )
}

#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    id: String,
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<ChunkUsage>,
    error: Option<ChunkError>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    content: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: usize,
    id: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    prompt_tokens_details: Option<PromptDetails>,
}

#[derive(Debug, Deserialize)]
struct PromptDetails {
    #[serde(default)]
    cached_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ChunkError {
    #[serde(default)]
    message: String,
    code: Option<Value>,
}

#[derive(Default)]
struct Decoder {
    // Keyed by the stream index: IDs may only appear on a call's first fragment.
    calls: BTreeMap<usize, ToolCallBuffer>,
    finish_reason: Option<String>,
    usage: Option<TokenUsage>,
    created: bool,
}

struct Handler {
    emitter: Option<EventEmitter>,
}

impl Handler {
    async fn finish(&mut self, decoder: &mut Decoder) {
        let Some(emitter) = self.emitter.take() else {
            return;
        };
        match decoder.finish_reason.as_deref() {
            Some("content_filter") => emitter.fail(ApiError::ContentFilter).await,
            // A length stop can cut tool arguments off; validation below rejects those.
            _ => {
                let calls = std::mem::take(&mut decoder.calls).into_values().collect();
                emitter.complete(calls, decoder.usage.take()).await;
            }
        }
    }
}

impl Handler {
    /// Returns false once the stream reached a terminal event.
    async fn handle(&mut self, decoder: &mut Decoder, event: Event) -> bool {
        if event.data.trim() == "[DONE]" {
            self.finish(decoder).await;
            return false;
        }
        let chunk: Chunk = match serde_json::from_str(&event.data) {
            Ok(chunk) => chunk,
            Err(error) => {
                tracing::debug!(%error, "ignoring an unreadable Chat Completions chunk");
                return true;
            }
        };
        let Some(emitter) = self.emitter.as_mut() else {
            return false;
        };
        if let Some(error) = chunk.error {
            let code = error.code.map(|code| match code {
                Value::String(code) => code,
                code => code.to_string(),
            });
            emitter
                .fail(stream_error(code.as_deref().unwrap_or(""), &error.message))
                .await;
            self.emitter = None;
            return false;
        }
        if !decoder.created {
            decoder.created = true;
            emitter.set_response_id(&chunk.id);
            if !emitter.created().await {
                return false;
            }
        }
        if let Some(reported) = chunk.usage {
            let cached = reported
                .prompt_tokens_details
                .map_or(0, |details| details.cached_tokens);
            decoder.usage = Some(usage(
                reported.prompt_tokens,
                cached,
                reported.completion_tokens,
            ));
        }
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                decoder.finish_reason = Some(reason);
            }
            if let Some(delta) = choice.delta.reasoning_content.as_deref()
                && !emitter.reasoning_delta(delta).await
            {
                return false;
            }
            if let Some(delta) = choice.delta.content.as_deref()
                && !emitter.text_delta(delta).await
            {
                return false;
            }
            for call in choice.delta.tool_calls {
                let buffer = decoder.calls.entry(call.index).or_default();
                if let Some(id) = call.id.filter(|id| !id.is_empty()) {
                    buffer.id = id;
                }
                if let Some(function) = call.function {
                    // Some services repeat the name on every fragment.
                    if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                        buffer.name = name;
                    }
                    if let Some(arguments) = function.arguments {
                        buffer.arguments.push_str(&arguments);
                    }
                }
            }
        }
        true
    }
}

pub(crate) fn spawn_stream(response: StreamResponse, idle_timeout: Duration) -> ResponseStream {
    let (tx, rx_event) = mpsc::channel(1600);
    tokio::spawn(async move {
        let mut decoder = Decoder::default();
        let mut handler = Handler {
            emitter: Some(EventEmitter::new(tx.clone())),
        };
        let mut stream = response.bytes.eventsource();
        loop {
            match next_event(&mut stream, idle_timeout, &tx).await {
                Next::Event(event) => {
                    if !handler.handle(&mut decoder, event).await {
                        return;
                    }
                }
                Next::ConsumerGone => return,
                Next::Failed(error) => {
                    if let Some(emitter) = handler.emitter.take() {
                        emitter.fail(error).await;
                    }
                    return;
                }
                // Services that skip `[DONE]` still report a finish reason; without one the
                // response is truncated and any buffered tool call must not run.
                Next::Closed if decoder.finish_reason.is_some() => {
                    handler.finish(&mut decoder).await;
                    return;
                }
                Next::Closed => {
                    if let Some(emitter) = handler.emitter.take() {
                        emitter.fail(ApiError::Stream(CLOSED_EARLY.to_string())).await;
                    }
                    return;
                }
            }
        }
    });
    ResponseStream {
        rx_event,
        upstream_request_id: None,
        interrupt: None,
    }
}
