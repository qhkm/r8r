# r8r Web UI

The web UI is compiled into `src/dashboard/static` and served by the Rust API server.

## Experience Goals

- Non-builder first: guided workflow setup from plain-language inputs
- Operational clarity: approvals, run status, and audit visibility in one dashboard
- Safe defaults: approval-aware templates and no hidden side effects
- Dual editor modes: `Guided` (default) and `Advanced Visual` (`/editor?mode=visual`)

## Advanced Visual Editor

The visual canvas (ReactFlow) supports:

- **Full node palette** — all 30 built-in node types (Core, Data, AI, Comms, Stores, Integration), each draggable onto the canvas. Nodes connect as `.depends_on` edges.
- **Open existing workflow** — lists workflows via `GET /api/workflows`, loads the chosen one via `GET /api/workflows/{name}` (definition parsed with `js-yaml`, including YAML or JSON payloads).
- **Trigger configuration** — add/remove manual, cron (schedule + timezone), webhook (path + method), and event (name + filter) triggers. Mirrors the `Trigger` enum in `src/workflow/types.rs`.
- **Node config** — per-node JSON config editor, node delete, colored node types.
- **Save** — `POST /api/workflows` upsert; definition string is JSON (a YAML subset, so stored workflows remain YAML/JSON-compatible for CLI and API consumers).

Converters live in `src/lib/workflowYaml.ts` (definition ↔ canvas state).

## Local Development

```bash
cd ui
pnpm install
pnpm dev
```

- UI dev server: `http://localhost:3000`
- API proxy target: `http://localhost:8080`

## Build For Embedded Dashboard

```bash
cd ui
pnpm build
```

This writes static assets to:

- `src/dashboard/static/index.html`
- `src/dashboard/static/assets/*`

The Rust server then serves these assets at `/` and `/editor`.
