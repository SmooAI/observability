/**
 * @smooai/observability-otel — OpenTelemetry foundation for @smooai/observability.
 *
 * Public surface:
 *   - `setupOtelSdk(options)` — Lambda / Node bootstrap. Sets up NodeSDK with
 *     OTLP/HTTP trace export + standard auto-instrumentations. Idempotent.
 *   - `readOtelCorrelation()` — read-only view of the active span's traceId /
 *     spanId / sampled flag, useful for embedding into other event shapes
 *     (e.g. logger formats, audit-log envelopes).
 *
 * **The bridge pattern (`bridgeClientToOtel`) is gone** — the Smoo core
 * Client now emits to OpenTelemetry natively on Node (see
 * `@smooai/observability/node`'s `registerOtelCapture`). There's nothing
 * to bridge: span events ARE the capture path.
 *
 * Usage (in a Lambda handler entry):
 *
 *   ```ts
 *   import { setupOtelSdk } from '@smooai/observability-otel';
 *   import { Client } from '@smooai/observability/node';
 *
 *   const otel = setupOtelSdk({
 *     serviceName: 'smoo-backend',
 *     environment: process.env.SST_STAGE,
 *     release: process.env.LAMBDA_FUNCTION_VERSION,
 *   });
 *
 *   Client.init({ dsn: 'https://api.smoo.ai/webhooks/observability/...' });
 *   // captureException now records on OTel spans automatically — no bridge call.
 *
 *   process.on('beforeExit', () => otel.flush());
 *   ```
 */

export { readOtelCorrelation } from './read-otel-context';
export { setupOtelSdk, _resetOtelSdkForTests } from './setup-otel-sdk';
export type { OtelSdkHandle, SetupOtelOptions } from './setup-otel-sdk';
