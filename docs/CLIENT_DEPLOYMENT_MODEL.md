# Client Deployment Model (First 10 Clients)

This is the recommended production model for onboarding and operating the first 10 client environments.

## Short Answer

- Yes, **Fargate runs containers**.
- You still build a Docker image.
- AWS manages the servers that run those containers.

## Why This Model

- Matches r8r's self-hosted direction (not edge/serverless-runtime-first).
- Gives per-client isolation without Kubernetes overhead.
- Keeps ops simple enough for a small team.

## Target Architecture

Use **one AWS region** and **one ECS cluster**, then isolate each client by service and data.

Per client:

1. One ECS service (`desiredCount=1`) on Fargate.
2. One task definition family (`r8r-<client>`).
3. One dedicated EFS access point for `/data` (SQLite + local files).
4. One Secrets Manager secret bundle (`R8R_API_KEY`, `R8R_MONITOR_TOKEN`, provider keys).
5. One DNS hostname + ALB host rule (`client-a.automations.example.com`).
6. One CloudWatch log group + alarms.

## Isolation Rules

1. Compute isolation: separate ECS service/task per client.
2. Data isolation: separate EFS access point and directory per client.
3. Secret isolation: separate secret per client, least-privilege task role.
4. API isolation: unique `R8R_API_KEY` per client.
5. Network isolation: private subnets for tasks; only ALB is public.

## Scaling Strategy

For first 10 clients:

1. Keep `desiredCount=1` per client service (SQLite write model is simplest with single active task).
2. Scale by task size first (CPU/memory).
3. Add autoscaling on CPU/memory only after stable baseline.
4. Use separate queue/worker architecture later if you outgrow single-task per client.

Suggested autoscaling profile (after baseline stabilizes):

1. `min_capacity=1`
2. `max_capacity=3`
3. `cpu_target=60`
4. `memory_target=75`

## Database Choice

For your current stage (first 10 clients), use:

1. **SQLite per client** (`/data/r8r.db`) on that client's EFS access point.
2. **Single active task per client** (`desiredCount=1`) to avoid multi-writer lock issues.
3. **Backups at storage layer** (EFS snapshots/backups).

Move to PostgreSQL when:

1. You need more than one active writer per client.
2. You need stronger reporting/query workloads across runs.
3. You start seeing SQLite contention under client load.

## Operations Baseline

1. Deployment: GitHub Actions build/push + ECS deploy per client.
2. Backups: AWS Backup for EFS (daily + retention policy).
3. Monitoring: 5xx alarm, restart loop alarm, high-latency alarm.
4. Rollback: keep previous task definition revision; one-click rollback.
5. Budgeting: tag all resources with `client_id`; create per-client cost budgets.

## Onboarding Checklist (Per Client)

1. Create client EFS access point.
2. Create client secrets (`R8R_API_KEY`, `R8R_MONITOR_TOKEN`).
3. Create/clone ECS service from template.
4. Add ALB host-based rule + DNS.
5. Run health check and one workflow smoke test.
6. Enable CloudWatch alarms and budget alerts.

## When To Re-Architect

Move beyond this model when one of these is true:

1. You need many concurrent workers per client.
2. You need shared multi-tenant control plane features (RBAC/tenant admin).
3. SQLite + EFS constraints become a bottleneck.

At that point, prioritize:

1. PostgreSQL storage backend.
2. Queue-based worker model.
3. Optional Kubernetes only if ECS operational limits become real.
