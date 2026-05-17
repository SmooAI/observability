# @smooai/observability

Core SDK for Smoo AI Observability. Captures unhandled errors in browser and Node, attaches user/release/environment context, and batches events to a Smoo ingest endpoint.

```
npm i @smooai/observability
```

See the [monorepo README](https://github.com/SmooAI/observability) for full usage. This package is universal: browser and Node both work, with the right entry resolved automatically by your bundler via the `exports` map.

## Status

`0.1.0` — types and client skeleton are stable. Capture handlers, breadcrumb wrappers, and stack parsers land incrementally in upcoming `0.x` releases. Track follow-ups in [SmooAI/smooai SMOODEV-1067](https://github.com/SmooAI/smooai).

## License

MIT
