/**
 * Universal entry point. Re-exports types and runtime-agnostic helpers.
 * Browser and Node entry points expose the runtime-specific `Client.init`.
 */
export * from './types';
export { Scope, withScope, getCurrentScope } from './scope';
export { Client } from './client';
// SMOODEV-1155: OTel GenAI semantic-convention helpers — apply `gen_ai.*`
// attributes to LLM/agent spans for the Smoo LLM dashboard + any
// GenAI-semconv-aware OTel backend (Datadog, Honeycomb, Phoenix, …).
export { setGenAIAttributes, recordGenAIMessage, type GenAIAttributes, type GenAIOperationName, type GenAISystem } from './gen-ai-attributes';
// SMOODEV-1155 follow-up: OpenAI Node SDK instrumentation. The OpenAI wire
// shape is what Groq / Together / DeepSeek / Azure / our LiteLLM gateway all
// speak, so one wrapper covers every provider reachable through that client.
export { wrapOpenAI, type WrapOpenAIOptions, type GenAICostInput } from './gen-ai-openai';
// ADR-097: session-scoped sampling, config-served telemetry settings, and W3C
// traceparent. Parity across the five SDKs is enforced by
// `parity/sampling-corpus.json` — see `parity/README.md`.
export {
    fnv1a32,
    sampleDecision,
    shouldEmitLog,
    normalizeLevel,
    parseLevel,
    meetsMinimumLevel,
    createDropCounter,
    LEVELS,
    type CanonicalLevel,
    type LogSamplingInput,
    type DropCounter,
} from './sampling';
export {
    DEFAULT_TELEMETRY_SETTINGS,
    TELEMETRY_SETTING_KEYS,
    resolveTelemetrySettings,
    loadTelemetrySettings,
    type TelemetrySettings,
    type TelemetrySettingsProvider,
} from './telemetry-settings';
export { parseTraceparent, formatTraceparent, type TraceContext } from './traceparent';
// PII scrubbing / hashing. `setPiiHashKey` MUST be called before anything is
// captured — without a key, emails / phones / addresses are redacted outright
// rather than hashed. On Node `bootstrapObservability` does it from
// SMOOAI_OBSERVABILITY_PII_HASH_KEY; the browser has no env, so call it here.
export { scrubString, scrubStringForOrg, scrubHeaders, scrubHeadersForOrg, setPiiHashKey, piiToken, type PiiKind } from './pii';
