package observability

import (
	"strings"

	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/trace"
)

// OTel GenAI semantic conventions — attribute helpers. Ports
// gen-ai-attributes.ts: a typed helper for setting gen_ai.* attributes on spans
// so any GenAI-semconv-aware OTel backend interprets them. Keys mirror the spec
// exactly.
//
// Spec: https://opentelemetry.io/docs/specs/semconv/gen-ai/

// GenAIOperationName is the gen_ai.operation.name value.
type GenAIOperationName string

const (
	GenAIOpChat           GenAIOperationName = "chat"
	GenAIOpTextCompletion GenAIOperationName = "text_completion"
	GenAIOpEmbeddings     GenAIOperationName = "embeddings"
	GenAIOpTool           GenAIOperationName = "tool"
	GenAIOpAgent          GenAIOperationName = "agent"
	GenAIOpRerank         GenAIOperationName = "rerank"
)

// GenAIAttributes is the full set of GenAI semconv attributes. All fields
// optional — set what you know. Pointer fields distinguish "unset" from a zero
// value (matching the TS optional-field semantics).
type GenAIAttributes struct {
	// System is the provider, e.g. "openai", "anthropic".
	System string
	// OperationName is the operation kind.
	OperationName GenAIOperationName

	// RequestModel is the model asked for.
	RequestModel string
	// ResponseModel is the model that responded (may differ under fallback).
	ResponseModel string
	// ResponseID is the provider-issued response id.
	ResponseID string

	Temperature *float64
	TopP        *float64
	TopK        *float64
	MaxTokens   *int64
	Seed        *int64

	UsageInputTokens  *int64
	UsageOutputTokens *int64
	UsageCachedTokens *int64
	UsageCostUSD      *float64

	// ToolNames is the list of tool names called within this span.
	ToolNames []string
	// Truncated is true if the response was cut off; nil = unset.
	Truncated *bool
	// FinishReason is the provider's reported finish reason.
	FinishReason string

	// EndUserID is the end-user id from your system (not the provider's).
	EndUserID string
	// ConversationID is the conversation/session id.
	ConversationID string
}

// SetGenAIAttributes applies a GenAIAttributes value onto a span. Skips unset
// fields so partial calls are idempotent. Never panics.
func SetGenAIAttributes(span trace.Span, attrs GenAIAttributes) {
	defer recoverSilently()
	if span == nil {
		return
	}

	var kvs []attribute.KeyValue
	add := func(kv attribute.KeyValue) { kvs = append(kvs, kv) }

	if attrs.System != "" {
		add(attribute.String("gen_ai.system", attrs.System))
	}
	if attrs.OperationName != "" {
		add(attribute.String("gen_ai.operation.name", string(attrs.OperationName)))
	}
	if attrs.RequestModel != "" {
		add(attribute.String("gen_ai.request.model", attrs.RequestModel))
	}
	if attrs.ResponseModel != "" {
		add(attribute.String("gen_ai.response.model", attrs.ResponseModel))
	}
	if attrs.ResponseID != "" {
		add(attribute.String("gen_ai.response.id", attrs.ResponseID))
	}
	if attrs.Temperature != nil {
		add(attribute.Float64("gen_ai.request.temperature", *attrs.Temperature))
	}
	if attrs.TopP != nil {
		add(attribute.Float64("gen_ai.request.top_p", *attrs.TopP))
	}
	if attrs.TopK != nil {
		add(attribute.Float64("gen_ai.request.top_k", *attrs.TopK))
	}
	if attrs.MaxTokens != nil {
		add(attribute.Int64("gen_ai.request.max_tokens", *attrs.MaxTokens))
	}
	if attrs.Seed != nil {
		add(attribute.Int64("gen_ai.request.seed", *attrs.Seed))
	}
	if attrs.UsageInputTokens != nil {
		add(attribute.Int64("gen_ai.usage.input_tokens", *attrs.UsageInputTokens))
	}
	if attrs.UsageOutputTokens != nil {
		add(attribute.Int64("gen_ai.usage.output_tokens", *attrs.UsageOutputTokens))
	}
	if attrs.UsageCachedTokens != nil {
		add(attribute.Int64("gen_ai.usage.cached_tokens", *attrs.UsageCachedTokens))
	}
	if attrs.UsageCostUSD != nil {
		add(attribute.Float64("gen_ai.usage.cost_usd", *attrs.UsageCostUSD))
	}
	if len(attrs.ToolNames) > 0 {
		add(attribute.StringSlice("gen_ai.tool.names", attrs.ToolNames))
	}
	if attrs.Truncated != nil {
		add(attribute.Bool("gen_ai.response.truncated", *attrs.Truncated))
	}
	if attrs.FinishReason != "" {
		add(attribute.String("gen_ai.response.finish_reason", attrs.FinishReason))
	}
	if attrs.EndUserID != "" {
		add(attribute.String("gen_ai.end_user.id", attrs.EndUserID))
	}
	if attrs.ConversationID != "" {
		add(attribute.String("gen_ai.conversation.id", attrs.ConversationID))
	}

	if len(kvs) > 0 {
		span.SetAttributes(kvs...)
	}
}

// GenAIMessageRole is the role for RecordGenAIMessage.
type GenAIMessageRole string

const (
	GenAIRoleUser      GenAIMessageRole = "user"
	GenAIRoleAssistant GenAIMessageRole = "assistant"
	GenAIRoleSystem    GenAIMessageRole = "system"
	GenAIRoleTool      GenAIMessageRole = "tool"
)

// GenAIMessageExtra carries optional tool linkage for RecordGenAIMessage.
type GenAIMessageExtra struct {
	ToolCallID string
	ToolName   string
}

// RecordGenAIMessage emits a gen_ai.{role}.message span event carrying the
// content, for the dashboard's prompt/completion side-by-side view. Use
// sparingly — these are size-heavy. Never panics.
func RecordGenAIMessage(span trace.Span, role GenAIMessageRole, content string, extra *GenAIMessageExtra) {
	defer recoverSilently()
	if span == nil {
		return
	}
	attrs := []attribute.KeyValue{attribute.String("gen_ai.message.content", content)}
	if extra != nil {
		if extra.ToolCallID != "" {
			attrs = append(attrs, attribute.String("gen_ai.tool_call.id", extra.ToolCallID))
		}
		if extra.ToolName != "" {
			attrs = append(attrs, attribute.String("gen_ai.tool.name", extra.ToolName))
		}
	}
	eventName := "gen_ai." + string(role) + ".message"
	// guard against a malformed role producing "gen_ai..message"
	if !strings.Contains(eventName, "..") {
		span.AddEvent(eventName, trace.WithAttributes(attrs...))
	}
}
