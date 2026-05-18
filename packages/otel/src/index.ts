/**
 * @smooai/observability-otel — OpenTelemetry foundation for @smooai/observability.
 *
 * Public surface:
 *   - `setupOtelSdk(options)` — Lambda / Node bootstrap. Sets up NodeSDK with
 *     OTLP/HTTP trace export + standard auto-instrumentations. Idempotent.
 *   - `bridgeClientToOtel()` — wraps the core Client so captureException also
 *     records on the active OTel span, and setUser/setTag flow through to
 *     span attributes. Use this to get one-click correlation between traces
 *     and Smoo error groups.
 *   - `readOtelCorrelation()` — read-only view of the active span's traceId /
 *     spanId / sampled flag, for embedding into other event shapes.
 *
 * Usage (in a Lambda handler entry):
 *
 *   ```ts
 *   import { setupOtelSdk, bridgeClientToOtel } from '@smooai/observability-otel';
 *   import { Client } from '@smooai/observability';
 *
 *   const otel = setupOtelSdk({
 *     serviceName: 'smoo-backend',
 *     environment: process.env.SST_STAGE,
 *     release: process.env.LAMBDA_FUNCTION_VERSION,
 *   });
 *
 *   Client.init({ dsn: 'https://api.smoo.ai/webhooks/observability/...' });
 *   bridgeClientToOtel();
 *
 *   process.on('beforeExit', () => otel.flush());
 *   ```
 */

export { bridgeClientToOtel, readOtelCorrelation, _resetBridgeForTests } from './bridge-to-client';
export type { BridgeOptions } from './bridge-to-client';
export { setupOtelSdk, _resetOtelSdkForTests } from './setup-otel-sdk';
export type { OtelSdkHandle, SetupOtelOptions } from './setup-otel-sdk';
