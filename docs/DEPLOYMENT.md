# Deployment Guide

This guide gives two fast paths:

1. Push a new workflow YAML from REPL output to a running cloud r8r instance.
2. Roll out a fresh r8r container image to a VPS/cloud VM.

`r8r` serves API + dashboard UI + `/editor` from the same service/container.

Prerequisites:

- For workflow push: `curl`, `jq` (and optionally `r8r` CLI for local validation).
- For VM rollout: `ssh` + `scp`.
- If Docker is already installed on the host, use `scripts/deploy-cloud.sh`.
- For a fresh Ubuntu/Debian VPS, use `scripts/deploy-vps.sh` (auto-installs Docker + Compose).
- If you are already SSH'ed into the VPS, use `scripts/deploy-on-vps.sh` (runs compose locally, no SSH hop).

## 1) REPL to Cloud Workflow Push

After you finish in REPL (`/export yaml out.yaml`), push that workflow:

```bash
chmod +x scripts/push-workflow.sh

scripts/push-workflow.sh \
  --server https://r8r.example.com \
  --file out.yaml
```

If your server uses API auth:

```bash
scripts/push-workflow.sh \
  --server https://r8r.example.com \
  --file out.yaml \
  --api-key "$R8R_API_KEY"
```

Push + run immediately:

```bash
scripts/push-workflow.sh \
  --server https://r8r.example.com \
  --file out.yaml \
  --execute \
  --input examples/input.json
```

## 2) VPS/Cloud VM Container Rollout (API + UI auto-start)

Use the provided cloud compose file:

- Compose: `deploy/docker-compose.cloud.yml`
- Deploy script: `scripts/deploy-cloud.sh`
- Fresh VPS wrapper: `scripts/deploy-vps.sh` (calls `deploy-cloud.sh --bootstrap`)

For AWS ECS/Fargate per-client deployments, use the Terraform scaffold in `infra/` and the isolation model in `docs/CLIENT_DEPLOYMENT_MODEL.md`.
Use `.github/workflows/terraform-infra.yml` for managed `plan/apply` from CI.

Terraform workflow secrets:

- Required: `AWS_ROLE_TO_ASSUME`
- Required for `apply`: `TF_STATE_BUCKET`
- Optional: `TF_STATE_LOCK_TABLE`, `TF_STATE_KEY_PREFIX`

Manual deploy on a host that already has Docker:

```bash
chmod +x scripts/deploy-cloud.sh

scripts/deploy-cloud.sh \
  --host 203.0.113.10 \
  --user ubuntu \
  --ssh-key ~/.ssh/id_ed25519 \
  --image ghcr.io/<owner>/r8r:<tag> \
  --monitor-token "<strong-random-token>" \
  --api-key "<optional-api-key>" \
  --cors-origins "https://workflows.example.com"
```

Manual deploy on a fresh Ubuntu/Debian VPS:

```bash
chmod +x scripts/deploy-vps.sh

scripts/deploy-vps.sh \
  --host 203.0.113.10 \
  --user ubuntu \
  --ssh-key ~/.ssh/id_ed25519 \
  --image ghcr.io/<owner>/r8r:<tag> \
  --monitor-token "<strong-random-token>" \
  --api-key "<optional-api-key>" \
  --cors-origins "https://workflows.example.com"
```

Both commands deploy one `r8r` service that includes:

1. API: `http://<host>:<port>/api/*`
2. Dashboard UI: `http://<host>:<port>/`
3. Guided editor UI: `http://<host>:<port>/editor`

## 3) Deploy Directly Inside the VPS (No SSH Hop)

Use this when you are already logged into the VPS shell.

```bash
chmod +x scripts/deploy-on-vps.sh

scripts/deploy-on-vps.sh \
  --image ghcr.io/<owner>/r8r:<tag> \
  --monitor-token "<strong-random-token>" \
  --api-key "<optional-api-key>" \
  --cors-origins "https://workflows.example.com"
```

Fresh Ubuntu/Debian VPS from inside the host:

```bash
scripts/deploy-on-vps.sh \
  --bootstrap \
  --image ghcr.io/<owner>/r8r:<tag> \
  --monitor-token "<strong-random-token>"
```

## GitHub Actions Deploy

Workflow: `.github/workflows/deploy-cloud.yml` (manual dispatch).

Required repository secrets:

- `GHCR_USERNAME`
- `GHCR_TOKEN`
- `DEPLOY_HOST`
- `DEPLOY_USER`
- `DEPLOY_SSH_KEY`
- `R8R_MONITOR_TOKEN`

Optional repository secrets:

- `DEPLOY_SSH_PORT`
- `R8R_API_KEY`
- `R8R_CORS_ORIGINS`

Optional workflow input:

- `bootstrap_vps` (default `false`) to auto-install Docker + Compose on Ubuntu/Debian VPS.

What it does:

1. Build and push image to `ghcr.io/<owner>/r8r:<tag>`.
2. SSH into your VM.
3. Upload cloud compose + `.env`.
4. Pull and restart the `r8r` container.
5. Verify `/api/health` and `/` on the remote host (unless `--skip-health-check`).

## Security Notes

- Set `R8R_MONITOR_TOKEN` in production.
- Set `R8R_API_KEY` to protect `/api/*`.
- Put r8r behind a reverse proxy for TLS, auth, and rate limiting.
- Keep deploy keys and registry tokens in GitHub secrets only.
