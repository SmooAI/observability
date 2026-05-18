# @smooai/observability-otel

OpenTelemetry foundation for [`@smooai/observability`](../core).

Sets up the OTel NodeSDK with OTLP/HTTP trace export and the standard auto-instrumentations bundle, then bridges the core `Client` so every `captureException` records on the active OTel span. The result: one-click correlation between traces, error groups, and logs — regardless of which logger you use.

## What you get

- **`setupOtelSdk(options)`** — Lambda / Node bootstrap. NodeSDK + OTLP/HTTP trace exporter + auto-instrumentations for HTTP / fetch / Postgres / Redis / etc. Idempotent.
- **`bridgeClientToOtel()`** — wraps `Client.captureException`, `Client.setUser`, `Client.setTag` so they also update the active OTel span. Exceptions become span events with `SpanStatusCode.ERROR`. If no span is active at capture time, a synthetic one is minted.
- **`readOtelCorrelation()`** — read the active span's `traceId` / `spanId` / sampled flag for embedding into other event shapes.

## Quick start

```ts
import { setupOtelSdk, bridgeClientToOtel } from '@smooai/observability-otel';
import { Client } from '@smooai/observability';

const otel = setupOtelSdk({
    serviceName: 'smoo-backend',
    environment: process.env.SST_STAGE,
    release: process.env.LAMBDA_FUNCTION_VERSION,
});

Client.init({ dsn: 'https://api.smoo.ai/webhooks/observability/ORG/TOKEN' });
bridgeClientToOtel();

process.on('beforeExit', () => otel.flush());
```

## Without `@smooai/logger`

This package depends on `@opentelemetry/api`, not on `@smooai/logger`. If you use winston, pino, bunyan, or console — pipe `readOtelCorrelation()` into your own log format and you get the same trace-id on logs / traces / errors.

```ts
import { readOtelCorrelation } from '@smooai/observability-otel';

logger.info('hello', { ...readOtelCorrelation() });
```

## Where this sits in the SDK

This is **Phase 1** of the OTel migration tracked under SMOODEV-1067c. Phase 2 swaps the ingest backend to accept OTLP/HTTP alongside the existing Smoo-native wire format. Phase 3 rebuilds the metrics SDK on OTel meters. Phase 4 turns `@smooai/logger`'s context into OTel baggage.

Until Phase 2 lands, `bridgeClientToOtel()` is additive — Smoo's existing transport still ships events to the backend; OTel becomes a parallel output for tracers.
