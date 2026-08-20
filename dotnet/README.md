# `SmooAI.Observability` — .NET

.NET SDK for SmooAI Observability, at feature parity with the TypeScript
reference SDK (`packages/core`): `Client.CaptureException` + breadcrumbs +
scoped context with a batched webhook transport (`System.Net.Http`), global
crash handlers, PII scrubbing with per-org HMAC hashing, OpenTelemetry traces +
metrics with M2M auth, `gen_ai.*` semantic-convention helpers, and ASP.NET Core
middleware. xUnit test suite runs in the `dotnet` lane of
[`pr-checks.yml`](../.github/workflows/pr-checks.yml).

See [`src/SmooAI.Observability/README.md`](src/SmooAI.Observability/README.md)
for usage.

## Status

✅ SDK implemented and tested ([SMOODEV-1159](https://smooai.atlassian.net/browse/SMOODEV-1159)).
**Not yet published to NuGet** — publishing is tag-triggered
(`dotnet-v<semver>` in [`publish.yml`](../.github/workflows/publish.yml)) and no
tag has shipped yet; until then, reference the project from source.
