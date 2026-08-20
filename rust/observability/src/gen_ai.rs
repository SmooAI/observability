//! OTel GenAI Semantic Conventions — attribute helpers.
//!
//! The OTel spec standardized `gen_ai.*` attributes for LLM / agent telemetry
//! so any OTel backend can interpret them with one shared vocabulary. This
//! module ports the TS `gen-ai-attributes.ts` helpers to set those attributes
//! on a live OTel [`opentelemetry::trace::Span`].
//!
//! Spec: <https://opentelemetry.io/docs/specs/semconv/gen-ai/>

use opentelemetry::trace::Span;
use opentelemetry::{Array, KeyValue, StringValue, Value};

/// GenAI operation kind. `agent` for top-level agent spans, `chat` for LLM
/// calls, `tool` for tool invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenAIOperationName {
    Chat,
    TextCompletion,
    Embeddings,
    Tool,
    Agent,
    Rerank,
}

impl GenAIOperationName {
    fn as_str(&self) -> &'static str {
        match self {
            GenAIOperationName::Chat => "chat",
            GenAIOperationName::TextCompletion => "text_completion",
            GenAIOperationName::Embeddings => "embeddings",
            GenAIOperationName::Tool => "tool",
            GenAIOperationName::Agent => "agent",
            GenAIOperationName::Rerank => "rerank",
        }
    }
}

/// Provider system. Known variants render their canonical spec value; `Other`
/// carries an arbitrary string so callers aren't blocked on the enum (mirrors
/// the TS `(string & {})` escape hatch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenAISystem {
    OpenAI,
    Anthropic,
    Groq,
    Gemini,
    Cohere,
    Mistral,
    Deepseek,
    AzureOpenAI,
    AwsBedrock,
    Other(String),
}

impl GenAISystem {
    fn as_str(&self) -> &str {
        match self {
            GenAISystem::OpenAI => "openai",
            GenAISystem::Anthropic => "anthropic",
            GenAISystem::Groq => "groq",
            GenAISystem::Gemini => "gemini",
            GenAISystem::Cohere => "cohere",
            GenAISystem::Mistral => "mistral",
            GenAISystem::Deepseek => "deepseek",
            GenAISystem::AzureOpenAI => "azure_openai",
            GenAISystem::AwsBedrock => "aws_bedrock",
            GenAISystem::Other(s) => s.as_str(),
        }
    }
}

/// Full set of GenAI semantic-convention attributes. All optional — set what
/// you know. Keys mirror the spec exactly so backends parse without mapping.
#[derive(Debug, Clone, Default)]
pub struct GenAIAttributes {
    pub system: Option<GenAISystem>,
    pub operation_name: Option<GenAIOperationName>,
    pub request_model: Option<String>,
    pub response_model: Option<String>,
    pub response_id: Option<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<f64>,
    pub max_tokens: Option<i64>,
    pub seed: Option<i64>,
    pub usage_input_tokens: Option<i64>,
    pub usage_output_tokens: Option<i64>,
    pub usage_cached_tokens: Option<i64>,
    pub usage_cost_usd: Option<f64>,
    pub tool_names: Option<Vec<String>>,
    pub truncated: Option<bool>,
    pub finish_reason: Option<String>,
    pub end_user_id: Option<String>,
    pub conversation_id: Option<String>,
}

/// Apply a [`GenAIAttributes`] value onto a live OTel span. Skips `None` fields
/// so partial calls (e.g. setting model + tokens after a streaming completion
/// finishes) are idempotent.
pub fn set_gen_ai_attributes<S: Span>(span: &mut S, attrs: &GenAIAttributes) {
    if let Some(v) = &attrs.system {
        span.set_attribute(KeyValue::new("gen_ai.system", v.as_str().to_string()));
    }
    if let Some(v) = attrs.operation_name {
        span.set_attribute(KeyValue::new("gen_ai.operation.name", v.as_str()));
    }
    if let Some(v) = &attrs.request_model {
        span.set_attribute(KeyValue::new("gen_ai.request.model", v.clone()));
    }
    if let Some(v) = &attrs.response_model {
        span.set_attribute(KeyValue::new("gen_ai.response.model", v.clone()));
    }
    if let Some(v) = &attrs.response_id {
        span.set_attribute(KeyValue::new("gen_ai.response.id", v.clone()));
    }
    if let Some(v) = attrs.temperature {
        span.set_attribute(KeyValue::new("gen_ai.request.temperature", v));
    }
    if let Some(v) = attrs.top_p {
        span.set_attribute(KeyValue::new("gen_ai.request.top_p", v));
    }
    if let Some(v) = attrs.top_k {
        span.set_attribute(KeyValue::new("gen_ai.request.top_k", v));
    }
    if let Some(v) = attrs.max_tokens {
        span.set_attribute(KeyValue::new("gen_ai.request.max_tokens", v));
    }
    if let Some(v) = attrs.seed {
        span.set_attribute(KeyValue::new("gen_ai.request.seed", v));
    }
    if let Some(v) = attrs.usage_input_tokens {
        span.set_attribute(KeyValue::new("gen_ai.usage.input_tokens", v));
    }
    if let Some(v) = attrs.usage_output_tokens {
        span.set_attribute(KeyValue::new("gen_ai.usage.output_tokens", v));
    }
    if let Some(v) = attrs.usage_cached_tokens {
        span.set_attribute(KeyValue::new("gen_ai.usage.cached_tokens", v));
    }
    if let Some(v) = attrs.usage_cost_usd {
        span.set_attribute(KeyValue::new("gen_ai.usage.cost_usd", v));
    }
    if let Some(names) = &attrs.tool_names {
        if !names.is_empty() {
            // A STRING ARRAY, matching the TS/Python/Go/.NET SDKs and the OTel
            // spec's array-valued attribute. This used to be `names.join(",")`,
            // which meant a Rust service's spans could not be filtered by tool
            // the way every other language's could — and a tool name containing
            // a comma silently became two tools.
            let values: Vec<StringValue> = names.iter().cloned().map(StringValue::from).collect();
            span.set_attribute(KeyValue::new(
                "gen_ai.tool.names",
                Value::Array(Array::String(values)),
            ));
        }
    }
    if let Some(v) = attrs.truncated {
        span.set_attribute(KeyValue::new("gen_ai.response.truncated", v));
    }
    if let Some(v) = &attrs.finish_reason {
        span.set_attribute(KeyValue::new("gen_ai.response.finish_reason", v.clone()));
    }
    if let Some(v) = &attrs.end_user_id {
        span.set_attribute(KeyValue::new("gen_ai.end_user.id", v.clone()));
    }
    if let Some(v) = &attrs.conversation_id {
        span.set_attribute(KeyValue::new("gen_ai.conversation.id", v.clone()));
    }
}

/// Message role for [`record_gen_ai_message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenAIRole {
    User,
    Assistant,
    System,
    Tool,
}

impl GenAIRole {
    fn as_str(&self) -> &'static str {
        match self {
            GenAIRole::User => "user",
            GenAIRole::Assistant => "assistant",
            GenAIRole::System => "system",
            GenAIRole::Tool => "tool",
        }
    }
}

/// Optional extras for a recorded GenAI message event.
#[derive(Debug, Clone, Default)]
pub struct GenAIMessageExtra {
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
}

/// Emit a `gen_ai.{role}.message` span event so the dashboard's
/// prompt/completion view can render the actual content. Use sparingly — these
/// are size-heavy.
///
/// Content is PII-scrubbed via [`crate::pii::scrub_string`] before it leaves the
/// process — prompts and tool arguments are the single most PII-dense payload
/// this SDK can touch, so raw content never reaches the wire. That covers both
/// classes: credentials (Bearer tokens, api keys, `password=`) are dropped, and
/// emails / phones / addresses are hashed per-org. Do not scrub inline here;
/// routing through the SDK's one scrub entry point is what makes the hashing
/// free.
///
/// ponytail: uses the org-less `scrub_string`, so hashes are salted with the
/// empty org — there is no org id in hand at this call site. Switch to
/// `scrub_string_for_org` if one ever reaches here. Matches the TS reference.
pub fn record_gen_ai_message<S: Span>(
    span: &mut S,
    role: GenAIRole,
    content: impl Into<String>,
    extra: &GenAIMessageExtra,
) {
    let mut kvs = vec![KeyValue::new(
        "gen_ai.message.content",
        crate::pii::scrub_string(&content.into()),
    )];
    if let Some(id) = &extra.tool_call_id {
        kvs.push(KeyValue::new("gen_ai.tool_call.id", id.clone()));
    }
    if let Some(name) = &extra.tool_name {
        kvs.push(KeyValue::new("gen_ai.tool.name", name.clone()));
    }
    span.add_event(format!("gen_ai.{}.message", role.as_str()), kvs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_strings_match_spec() {
        assert_eq!(GenAISystem::Anthropic.as_str(), "anthropic");
        assert_eq!(GenAISystem::AwsBedrock.as_str(), "aws_bedrock");
        assert_eq!(GenAISystem::Other("custom".into()).as_str(), "custom");
    }

    #[test]
    fn operation_names_match_spec() {
        assert_eq!(
            GenAIOperationName::TextCompletion.as_str(),
            "text_completion"
        );
        assert_eq!(GenAIOperationName::Agent.as_str(), "agent");
    }

    #[test]
    fn role_strings_match_spec() {
        assert_eq!(GenAIRole::Assistant.as_str(), "assistant");
    }
}
