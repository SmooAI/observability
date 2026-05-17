---
'@smooai/observability': minor
---

Browser capture MVP. Wires up `window.onerror` + `unhandledrejection` global handlers, optional `console.error` tap, `fetch` + navigation breadcrumb wrappers, batched `fetch` transport with `navigator.sendBeacon` flush on `pagehide`/`visibilitychange`, PII scrubbing (Bearer tokens, password/token/api-key params, OpenAI-style `sk-...` keys, sensitive headers), and an engine-agnostic V8 + Spidermonkey stack parser. `Client.init` now auto-installs everything when called from the browser entry. SDK-internal frames are stripped from captured stacks. `Error.cause` chains are walked into the exception envelope.
