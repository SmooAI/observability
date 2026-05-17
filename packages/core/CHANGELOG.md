# @smooai/observability

## 0.2.0

### Minor Changes

- 40bbb38: Browser capture MVP. Wires up `window.onerror` + `unhandledrejection` global handlers, optional `console.error` tap, `fetch` + navigation breadcrumb wrappers, batched `fetch` transport with `navigator.sendBeacon` flush on `pagehide`/`visibilitychange`, PII scrubbing (Bearer tokens, password/token/api-key params, OpenAI-style `sk-...` keys, sensitive headers), and an engine-agnostic V8 + Spidermonkey stack parser. `Client.init` now auto-installs everything when called from the browser entry. SDK-internal frames are stripped from captured stacks. `Error.cause` chains are walked into the exception envelope.
- ebda331: Initial 0.1.0 release. Universal browser + Node core with React and Next.js wrappers. Capture handlers and transport land incrementally — track follow-ups in [SmooAI/smooai SMOODEV-1067](https://github.com/SmooAI/smooai).

## 0.1.0

### Minor Changes

- Initial release. Universal browser + Node SDK skeleton with `Client.init`, `captureException`, `captureMessage`, `Scope` / `withScope`, breadcrumbs, and full TypeScript types covering the Sentry-shaped event envelope. Capture handlers, transport, and stack parsers land incrementally — see follow-up issues in [SmooAI/smooai](https://github.com/SmooAI/smooai) under SMOODEV-1067.
