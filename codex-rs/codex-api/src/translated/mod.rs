//! Adapters for model protocols other than the Responses API.
//!
//! The rest of the engine speaks `ResponseItem`/`ResponseEvent`. Each adapter encodes a
//! `ResponsesApiRequest` for its protocol and decodes the stream back into those events, so
//! tools, approvals and history behave the same for every provider. Only plain JSON function
//! tools are translated: freeform, namespaced and hosted tools have no faithful equivalent
//! here and are left out rather than approximated.
//!
//! A tool call is emitted only after its complete arguments parsed as a JSON object. Truncated
//! or invalid arguments fail the stream, so partial JSON can never reach tool execution.

pub(crate) mod anthropic;
pub(crate) mod chat;
mod sse;

#[cfg(test)]
mod tests;

use crate::auth::AuthError;
use crate::auth::AuthProvider;
use crate::auth::SharedAuthProvider;
use crate::common::ResponseEvent;
use crate::common::ResponsesApiRequest;
use crate::error::ApiError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use http::HeaderMap;
use http::HeaderValue;
use http::header::AUTHORIZATION;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub(crate) const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";
pub(crate) const ANTHROPIC_MESSAGES_PATH: &str = "/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Function tool extracted from the Responses tool list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FunctionTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

pub(crate) fn function_tools(request: &ResponsesApiRequest) -> Vec<FunctionTool> {
    let Some(tools) = request.tools.as_ref() else {
        return Vec::new();
    };
    let tools: Vec<Value> = match serde_json::from_str(tools.as_raw_value().get()) {
        Ok(tools) => tools,
        Err(error) => {
            tracing::warn!(%error, "tool list could not be translated");
            return Vec::new();
        }
    };
    tools
        .into_iter()
        .filter_map(|tool| {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                tracing::warn!(
                    tool_type = tool.get("type").and_then(Value::as_str),
                    "tool has no equivalent in this model protocol and was left out"
                );
                return None;
            }
            Some(FunctionTool {
                name: tool.get("name")?.as_str()?.to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parameters: tool
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            })
        })
        .collect()
}

/// Text of a tool result. Images and audio cannot be carried by these protocols' tool results.
pub(crate) fn output_text(output: &FunctionCallOutputPayload) -> String {
    match &output.body {
        FunctionCallOutputBody::Text(text) => text.clone(),
        FunctionCallOutputBody::ContentItems(items) => items
            .iter()
            .filter_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// One piece of a conversation message in protocol-neutral form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MessagePart {
    Text(String),
    ImageUrl(String),
}

pub(crate) fn message_parts(content: &[ContentItem]) -> Vec<MessagePart> {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text, .. } | ContentItem::OutputText { text, .. } => {
                Some(MessagePart::Text(text.clone()))
            }
            ContentItem::InputImage {
                image: ImageReference::Inline { image_url },
                ..
            } => Some(MessagePart::ImageUrl(image_url.clone())),
            _ => None,
        })
        .collect()
}

pub(crate) fn reasoning_text(item: &ResponseItem) -> Option<String> {
    let ResponseItem::Reasoning { content, .. } = item else {
        return None;
    };
    let text = serde_json::to_value(content.as_ref()?).ok()?;
    let text = text
        .as_array()?
        .iter()
        .filter_map(|part| part.get("text")?.as_str())
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

/// Anthropic authenticates with `x-api-key`; provider auth resolves to a bearer token.
struct AnthropicAuth(SharedAuthProvider);

impl AuthProvider for AnthropicAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) -> Result<(), AuthError> {
        let mut resolved = HeaderMap::new();
        self.0.add_auth_headers(&mut resolved)?;
        if let Some(value) = resolved.remove(AUTHORIZATION) {
            let value = value
                .to_str()
                .map_err(|_| AuthError::Build("API key is not valid header text".to_string()))?;
            let key = value.strip_prefix("Bearer ").unwrap_or(value);
            let mut key = HeaderValue::from_str(key)
                .map_err(|_| AuthError::Build("API key is not valid header text".to_string()))?;
            key.set_sensitive(true);
            headers.insert("x-api-key", key);
        }
        headers.extend(resolved);
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        Ok(())
    }
}

pub fn anthropic_auth(auth: SharedAuthProvider) -> SharedAuthProvider {
    Arc::new(AnthropicAuth(auth))
}

/// Tool call being assembled from stream fragments.
#[derive(Debug, Default, Clone)]
pub(crate) struct ToolCallBuffer {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

impl ToolCallBuffer {
    /// Normalized arguments, or an error naming the call. Nothing runs when this fails.
    fn validated_arguments(&self) -> Result<String, ApiError> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err(ApiError::Stream(
                "The model sent a tool call without an ID or name; nothing was executed."
                    .to_string(),
            ));
        }
        let arguments = if self.arguments.trim().is_empty() {
            "{}"
        } else {
            self.arguments.as_str()
        };
        match serde_json::from_str::<Value>(arguments) {
            Ok(Value::Object(_)) => Ok(arguments.to_string()),
            _ => Err(ApiError::Stream(format!(
                "The model sent incomplete or invalid arguments for tool `{}`; nothing was executed.",
                self.name
            ))),
        }
    }
}

/// Turns decoded protocol fragments into the event order the engine expects.
pub(crate) struct EventEmitter {
    tx: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    response_id: String,
    message_id: Option<String>,
    text: String,
    reasoning_id: Option<String>,
    reasoning: String,
}

impl EventEmitter {
    pub(crate) fn new(tx: mpsc::Sender<Result<ResponseEvent, ApiError>>) -> Self {
        Self {
            tx,
            response_id: format!("resp_{}", Uuid::new_v4().simple()),
            message_id: None,
            text: String::new(),
            reasoning_id: None,
            reasoning: String::new(),
        }
    }

    pub(crate) fn set_response_id(&mut self, id: &str) {
        if !id.is_empty() {
            self.response_id = id.to_string();
        }
    }

    async fn send(&self, event: ResponseEvent) -> bool {
        self.tx.send(Ok(event)).await.is_ok()
    }

    pub(crate) async fn fail(&self, error: ApiError) {
        let _ = self.tx.send(Err(error)).await;
    }

    pub(crate) async fn created(&self) -> bool {
        self.send(ResponseEvent::Created {
            response_id: Some(self.response_id.clone()),
        })
        .await
    }

    pub(crate) async fn reasoning_delta(&mut self, delta: &str) -> bool {
        if delta.is_empty() {
            return true;
        }
        if self.reasoning_id.is_none() {
            let id = format!("rs_{}", Uuid::new_v4().simple());
            let Some(item) = reasoning_item(&id, "") else {
                return true;
            };
            self.reasoning_id = Some(id);
            if !self.send(ResponseEvent::OutputItemAdded(item)).await {
                return false;
            }
        }
        self.reasoning.push_str(delta);
        self.send(ResponseEvent::ReasoningContentDelta {
            delta: delta.to_string(),
            content_index: 0,
        })
        .await
    }

    pub(crate) async fn text_delta(&mut self, delta: &str) -> bool {
        if delta.is_empty() {
            return true;
        }
        if !self.finish_reasoning().await {
            return false;
        }
        if self.message_id.is_none() {
            let id = format!("msg_{}", Uuid::new_v4().simple());
            let Some(item) = message_item(&id, "") else {
                return true;
            };
            self.message_id = Some(id);
            if !self.send(ResponseEvent::OutputItemAdded(item)).await {
                return false;
            }
        }
        self.text.push_str(delta);
        self.send(ResponseEvent::OutputTextDelta(delta.to_string()))
            .await
    }

    async fn finish_reasoning(&mut self) -> bool {
        let Some(id) = self.reasoning_id.take() else {
            return true;
        };
        let text = std::mem::take(&mut self.reasoning);
        match reasoning_item(&id, &text) {
            Some(item) => self.send(ResponseEvent::OutputItemDone(item)).await,
            None => true,
        }
    }

    /// Validates every call before emitting anything, so a bad call cannot leave a sibling
    /// call running while the turn fails.
    pub(crate) async fn complete(
        mut self,
        tool_calls: Vec<ToolCallBuffer>,
        token_usage: Option<TokenUsage>,
    ) {
        let mut calls = Vec::with_capacity(tool_calls.len());
        for call in &tool_calls {
            match call.validated_arguments() {
                Ok(arguments) => calls.push((call, arguments)),
                Err(error) => {
                    self.fail(error).await;
                    return;
                }
            }
        }
        if !self.finish_reasoning().await {
            return;
        }
        if let Some(id) = self.message_id.take() {
            let text = std::mem::take(&mut self.text);
            if let Some(item) = message_item(&id, &text)
                && !self.send(ResponseEvent::OutputItemDone(item)).await
            {
                return;
            }
        }
        for (call, arguments) in calls {
            let item = function_call_item(call, &arguments);
            if let Some(item) = item
                && !self.send(ResponseEvent::OutputItemDone(item)).await
            {
                return;
            }
        }
        let _ = self
            .send(ResponseEvent::Completed {
                response_id: self.response_id.clone(),
                token_usage,
                usage_metadata: None,
                end_turn: None,
            })
            .await;
    }
}

// Items are built through their wire shape so they stay valid as `ResponseItem` gains
// optional fields.
fn item_from_json(value: Value) -> Option<ResponseItem> {
    match serde_json::from_value(value) {
        Ok(item) => Some(item),
        Err(error) => {
            tracing::error!(%error, "failed to build a response item");
            None
        }
    }
}

fn message_item(id: &str, text: &str) -> Option<ResponseItem> {
    item_from_json(json!({
        "type": "message",
        "id": id,
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text }],
    }))
}

fn reasoning_item(id: &str, text: &str) -> Option<ResponseItem> {
    item_from_json(json!({
        "type": "reasoning",
        "id": id,
        "summary": [],
        "content": [{ "type": "reasoning_text", "text": text }],
        "encrypted_content": null,
    }))
}

fn function_call_item(call: &ToolCallBuffer, arguments: &str) -> Option<ResponseItem> {
    item_from_json(json!({
        "type": "function_call",
        "name": call.name,
        "arguments": arguments,
        "call_id": call.id,
    }))
}

pub(crate) fn usage(input: i64, cached: i64, output: i64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        total_tokens: input + output,
        ..TokenUsage::default()
    }
}

/// Classifies an error reported inside a stream; HTTP failures never reach this point.
pub(crate) fn stream_error(code: &str, message: &str) -> ApiError {
    let lower = format!("{code} {message}").to_ascii_lowercase();
    if lower.contains("context_length")
        || lower.contains("prompt is too long")
        || lower.contains("too many tokens")
    {
        ApiError::ContextWindowExceeded
    } else if lower.contains("rate_limit") || lower.contains("overloaded") || code == "429" {
        ApiError::RateLimit(message.to_string())
    } else if lower.contains("authentication") || lower.contains("invalid_api_key") {
        ApiError::InvalidRequest {
            message: format!("The provider rejected the API key: {message}"),
        }
    } else {
        ApiError::Stream(format!("provider error: {message}"))
    }
}
