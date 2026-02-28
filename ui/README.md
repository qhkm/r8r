# r8r Web UI

The web UI is compiled into `src/dashboard/static` and served by the Rust API server.

## Experience Goals

- Non-builder first: guided workflow setup from plain-language inputs
- Operational clarity: approvals, run status, and audit visibility in one dashboard
- Safe defaults: approval-aware templates and no hidden side effects
- Dual editor modes: `Guided` (default) and `Advanced Visual` (`/editor?mode=visual`)

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
