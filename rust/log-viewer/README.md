# SmooAI Observability Studio (`smooobs`)

[![Crate: `smooai-observability-viewer`](https://img.shields.io/badge/crate-smooai--observability--viewer-blue)](#)

A native Rust desktop client for the entire SmooAI observability stack: structured logs, errors, metrics, distributed traces, gen-AI/LLM telemetry — viewable both **locally** (against `.smooai-logs/` files on disk) and **remotely** (against `api.smoo.ai` over M2M `client_credentials`).

Built on `eframe` + `egui`. Single signed binary on macOS, Windows, Linux.

---

## Status

| Phase | Scope                                                       | Status              |
| ----- | ----------------------------------------------------------- | ------------------- |
| 0     | Plan + ADR                                                  | Done (SMOODEV-1175) |
| 1     | Move crate from `~/dev/smooai/logger/log-viewer/`, scaffold | Done                |
| 2     | M2M auth + API client + Settings tab                        | Planned             |
| 3     | Remote Logs view                                            | Planned             |
| 4     | Remote Errors view (list + detail + stack frames)           | Planned             |
| 5     | Remote Metrics view (mean / percentiles / heatmap)          | Planned             |
| 6     | Polish — command palette, persisted state, deep links       | Planned             |
| 7     | Distribution — signed .dmg / .msi / AppImage + auto-update  | Planned             |
| 8     | Traces view (blocked on SMOODEV-1161)                       | Planned             |
| 9     | LLM/gen-AI view (blocked on SMOODEV-1160)                   | Planned             |

The full plan lives at [`docs/Engineering/Rust-Desktop-Observability-Viewer.md`](https://github.com/SmooAI/smooai/blob/main/docs/Engineering/Rust-Desktop-Observability-Viewer.md) inside the smooai monorepo. Decision rationale: [ADR-013](https://github.com/SmooAI/smooai/blob/main/docs/Decisions/ADR-013-Native-Desktop-Observability-Viewer.md).

---

## Run locally

```bash
cd ~/dev/smooai/observability/rust
cargo run --release -p smooai-observability-viewer
```

This opens the viewer in **local-only** mode against `.smooai-logs/` directories under a folder you pick on first launch.

---

## Crate layout

```
rust/log-viewer/
├── Cargo.toml               # binary name: smooobs
├── README.md                # this file (product framing)
├── ABOUT.md                 # developer onboarding (local-pipeline internals)
├── assets/
│   ├── app-icon.png
│   └── smoo-logo.png
└── src/
    ├── main.rs              # eframe entry + local pipeline (phase 1)
    ├── theme.rs             # brand colors + egui visuals
    ├── source/              # DataSource abstraction (local + remote)
    ├── auth/                # OS keychain + client_credentials exchange
    ├── api/                 # typed client for api.smoo.ai observability routes
    │   ├── logs.rs
    │   ├── errors.rs
    │   ├── metrics.rs
    │   └── connections.rs
    ├── view/                # per-dashboard panels (logs/errors/metrics/...)
    └── widgets/             # reusable heatmap, KPI cards, stack-frame viewer
```

Most module trees are stubs as of phase 1; they'll be filled in by phases 2–6.

---

## Predecessor

This crate descends from the standalone `smooai-log-viewer` that used to live in [`SmooAI/logger`](https://github.com/SmooAI/logger). The old location is now a deprecation pointer — see [its README](https://github.com/SmooAI/logger/tree/main/log-viewer).

---

## License

MIT. See `LICENSE` in the repo root.
