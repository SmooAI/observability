---
'@smooai/observability': minor
---

SMOODEV-1148: Node Client.captureException now fires BOTH OTel capture AND HTTP webhook transport.

Previously the runtime-native captureHandler (OTel span events) short-circuited the HTTP transport, so Node errors never reached the webhook-backed Errors dashboard. Now both paths fire: OTel keeps emitting span events for tracing/observability, and the webhook also gets the event for the Errors UI.

Node init now registers an HTTP transport (`makeNodeTransport`) when a `dsn` is configured. No-op when DSN is empty.
