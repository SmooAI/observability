---
'@smooai/observability': minor
---

Add `installWebVitals(metrics, opts?)` browser RUM one-liner. Wires the
`web-vitals` library into a Smoo `MetricsClient` to record Core Web Vitals
(LCP/CLS/INP/FCP/TTFB) as `web.vitals.*` metrics with `route` / `rating` /
`navigation_type` attributes. Browser-guarded, idempotent, lazy-loads
`web-vitals` only when called. Exported from `@smooai/observability/browser`.
