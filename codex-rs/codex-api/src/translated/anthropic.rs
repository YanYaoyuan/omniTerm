//! Native Anthropic Messages adapter, including `tool_use`/`tool_result` content blocks.

use super::EventEmitter;
use super::MessagePart;
use super::ToolCallBuffer;
use super::function_tools;
use super::message_parts;
use super::output_text;
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
use eventsource_stream::Event;
use eventsource_stream::Eventsource;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;

const MAX_OUTPUT_TOKENS: u32 = 8192;

pub(crate) fn encode_request(request: &ResponsesApiRequest) -> Value {
    let mut system: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut push = |role: &str, block: Value| {
        // The API needs alternating roles, so adjacent blocks share one message.
        match messages.last_mut() {
            Some(message) if message["role"] == role => {
                if let Some(content) = message["content"].as_array_mut() {
                    content.push(block);
                }
            }
            _ => messages.push(json!({ "role": role, "content": [block] })),
        }
    };
    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let parts = message_parts(content);
                if matches!(role.as_str(), "developer" | "system") {
                    system.extend(parts.into_iter().filter_map(|part| match part {
                        MessagePart::Text(text) => Some(text),
                        MessagePart::ImageUrl(_) => None,
                    }));
                    continue;
                }
                let role = if role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                for part in parts {
                    match part {
                        MessagePart::Text(text) if text.is_empty() => {}
                        MessagePart::Text(text) => {
                            push(role, json!({ "type": "text", "text": text }));
                        }
                        MessagePart::ImageUrl(url) => {
                            if let Some(image) = image_block(&url) {
                                push(role, image);
                            }
                        }
                    }
                }
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                let input = serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                push(
                    "assistant",
                    json!({ "type": "tool_use", "id": call_id, "name": name, "input": input }),
                );
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": output_text(output),
                });
                if output.success == Some(false) {
                    block["is_error"] = json!(true);
                }
                push("user", block);
            }
            _ => {}
        }
    }
    let tools: Vec<Value> = function_tools(request)
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters,
            })
        })
        .collect();
    let mut body = json!({
        "model": request.model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "messages": messages,
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        if !request.parallel_tool_calls {
            body["tool_choice"] = json!({ "type": "auto", "disable_parallel_tool_use": true });
        }
    }
    body
}

/// Only inline images can be sent; the protocol has no `image_url` content block.
fn image_block(url: &str) -> Option<Value> {
    let (header, data) = url.strip_prefix("data:")?.split_once(',')?;
    let media_type = header.strip_suffix(";base64")?;
    Some(json!({
        "type": "image",
        "source": { "type": "base64", "media_type": media_type, "data": data },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    MessageStart {
        message: MessageStart,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: BlockDelta,
    },
    MessageDelta {
        #[serde(default)]
        delta: MessageDelta,
        usage: Option<Usage>,
    },
    MessageStop,
    Error {
        error: ErrorBody,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct MessageStart {
    #[serde(default)]
    id: String,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BlockDelta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Default, Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
}

#[derive(Default)]
struct Decoder {
    calls: BTreeMap<usize, ToolCallBuffer>,
    stop_reason: Option<String>,
    input_tokens: i64,
    cached_tokens: i64,
    output_tokens: i64,
}

struct Handler {
    emitter: Option<EventEmitter>,
}

impl Handler {
    async fn finish(&mut self, decoder: &mut Decoder) {
        let Some(emitter) = self.emitter.take() else {
            return;
        };
        if decoder.stop_reason.as_deref() == Some("refusal") {
            emitter.fail(ApiError::ContentFilter).await;
            return;
        }
        let calls = std::mem::take(&mut decoder.calls).into_values().collect();
        let input = decoder.input_tokens + decoder.cached_tokens;
        emitter
            .complete(
                calls,
                Some(usage(input, decoder.cached_tokens, decoder.output_tokens)),
            )
            .await;
    }
}

impl Handler {
    /// Returns false once the stream reached a terminal event.
    async fn handle(&mut self, decoder: &mut Decoder, event: Event) -> bool {
        let parsed: StreamEvent = match serde_json::from_str(&event.data) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::debug!(%error, "ignoring an unreadable Messages event");
                return true;
            }
        };
        let Some(emitter) = self.emitter.as_mut() else {
            return false;
        };
        match parsed {
            StreamEvent::MessageStart { message } => {
                emitter.set_response_id(&message.id);
                if let Some(reported) = message.usage {
                    decoder.input_tokens = reported.input_tokens;
                    decoder.cached_tokens = reported.cache_read_input_tokens;
                }
                return emitter.created().await;
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block: ContentBlock::ToolUse { id, name },
            } => {
                decoder.calls.insert(
                    index,
                    ToolCallBuffer {
                        id,
                        name,
                        arguments: String::new(),
                    },
                );
            }
            StreamEvent::ContentBlockDelta { index, delta } => match delta {
                BlockDelta::TextDelta { text } => return emitter.text_delta(&text).await,
                BlockDelta::ThinkingDelta { thinking } => {
                    return emitter.reasoning_delta(&thinking).await;
                }
                BlockDelta::InputJsonDelta { partial_json } => {
                    if let Some(call) = decoder.calls.get_mut(&index) {
                        call.arguments.push_str(&partial_json);
                    }
                }
                BlockDelta::Other => {}
            },
            StreamEvent::MessageDelta { delta, usage } => {
                if let Some(reason) = delta.stop_reason {
                    decoder.stop_reason = Some(reason);
                }
                if let Some(reported) = usage {
                    decoder.output_tokens = reported.output_tokens;
                }
            }
            StreamEvent::MessageStop => {
                self.finish(decoder).await;
                return false;
            }
            StreamEvent::Error { error } => {
                emitter.fail(stream_error(&error.kind, &error.message)).await;
                self.emitter = None;
                return false;
            }
            StreamEvent::ContentBlockStart { .. } | StreamEvent::Other => {}
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
            let error = match next_event(&mut stream, idle_timeout, &tx).await {
                Next::Event(event) => {
                    if handler.handle(&mut decoder, event).await {
                        continue;
                    }
                    return;
                }
                Next::ConsumerGone => return,
                Next::Failed(error) => error,
                Next::Closed => ApiError::Stream(CLOSED_EARLY.to_string()),
            };
            if let Some(emitter) = handler.emitter.take() {
                emitter.fail(error).await;
            }
            return;
        }
    });
    ResponseStream {
        rx_event,
        upstream_request_id: None,
        interrupt: None,
    }
}
