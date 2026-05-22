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
