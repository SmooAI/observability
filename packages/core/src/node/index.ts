/**
 * Node entry — Lambda / long-running Node services.
 *
 * **OTel-first**: `Client.init` on Node wires the OpenTelemetry-native
 * capture path (`registerOtelCapture`) — every captured exception becomes
 * a span event on the active OTel span (or a synthetic one) with status
 * ERROR and OTLP-shaped attributes. The OpenTelemetry SDK handles
 * batching, retry, and wire format; the Smoo SDK does NOT spin up its own
 * HTTP transport on Node.
 *
 * For consumers who haven't initialized OTel yet (no global TracerProvider),
 * the OTel API quietly no-ops — events are dropped rather than crashing.
 * Use `@smooai/observability-otel/setupOtelSdk()` (recommended) or your own
 * OTel NodeSDK bootstrap before calling `Client.init`.
 *
 * The Hono middleware + process-level error handlers are exported separately
 * so consumers wire them on their app explicitly. Browser-only integrations
 * (DOM breadcrumbs etc.) are NOT imported here so the node bundle stays small.
 *
 * `@smooai/logger` is optional. When present, its CONTEXT global feeds OTel
 * baggage (handled elsewhere — see `@smooai/observability-otel`). When
 * absent, OTel ambient context is the single source of correlation truth.
 */
import { Client } from '../client';
import { registerNodeGlobalHandlers } from './global-handlers';
import { registerOtelCapture } from './otel-capture';
import { makeNodeTransport } from './transport';

export { Client, Scope, withScope, getCurrentScope } from '../index';
export * from '../types';
export { parseStack } from '../stack-parser';
export { registerNodeGlobalHandlers, _resetNodeGlobalHandlersForTests } from './global-handlers';
export { registerOtelCapture, _resetOtelCaptureForTests } from './otel-capture';
export { observabilityMiddleware } from './middleware';
export type { ObservabilityMiddlewareOptions } from './middleware';
export { makeNodeTransport } from './transport';
export {
    setGenAIAttributes,
    recordGenAIMessage,
    type GenAIAttributes,
    type GenAIOperationName,
    type GenAISystem,
} from '../gen-ai-attributes';

// Auto-wire on init — Node is OTel-first. Set `autoInstrumentation: false`
// to opt out of process error handlers; the OTel capture path is always
// registered when the SDK is initialized.
//
// SMOODEV-1148: also register an HTTP transport when a `dsn` is configured
// so captureException fans out to BOTH the OTel-native capture (span events)
// AND the webhook POST (errorEvents table → Errors dashboard). Without this,
// Node errors never reach the Errors UI even though the SDK is correctly
// initialized.
const originalInit = Client.init.bind(Client);
Client.init = (options) => {
    originalInit(options);
    registerOtelCapture();
    if (options.autoInstrumentation !== false) {
        registerNodeGlobalHandlers({ exitOnUncaught: false });
    }
    if (options.dsn) {
        const transport = makeNodeTransport(options);
        Client._registerTransport(async (batch) => {
            for (const evt of batch) transport.enqueue(evt);
        });
    }
};
