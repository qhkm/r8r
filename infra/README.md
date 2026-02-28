# Terraform Baseline (Per-Client ECS/Fargate)

This folder is a starter baseline for deploying r8r per client on AWS ECS/Fargate.

## What This Creates

- One VPC with public + private subnets.
- One ECS cluster.
- One ALB with HTTP (`80`) and HTTPS (`443`) listeners.
- One shared EFS filesystem.
- Per-client ECS service + task definition.
- Per-client EFS access point (`/clients/<client_id>`).
- Per-client API key + monitor token in Secrets Manager.
- Optional per-client ECS autoscaling policies (CPU + memory target tracking).

## Quick Start

1. Copy vars:

```bash
cd infra
cp terraform.tfvars.example terraform.tfvars
```

2. Edit client entries in `terraform.tfvars`.

Required for managed HTTPS:

- `route53_zone_id` (hosted zone for your client domains), or
- `acm_certificate_arn` (if reusing an existing cert).

3. Run Terraform:

```bash
terraform init
terraform plan
terraform apply
```

## HTTPS / DNS

- `enable_https=true` creates ALB `443` listener.
- If `acm_certificate_arn` is empty and `route53_zone_id` is set, Terraform creates ACM cert + DNS validation records.
- If `create_client_dns_records=true`, Terraform creates Route53 alias records for each client domain.

## Autoscaling

Set per client:

```hcl
autoscaling = {
  enabled       = true
  min_capacity  = 1
  max_capacity  = 3
  cpu_target    = 60
  memory_target = 75
}
```

If autoscaling is enabled, Terraform ignores ongoing `desired_count` drift because App Auto Scaling controls it.

## Database Strategy (Now vs Later)

For first 10 clients, use:

- SQLite per client file at `/data/r8r.db`
- One active task per client (`desired_count=1`)
- EFS access point per client for isolation and persistence

When to move to PostgreSQL:

- You need multiple active writers per client
- SQLite lock contention starts impacting latency
- You need stronger cross-service querying/reporting

## Notes

- Restrict IAM resource scopes before production hardening.
- Keep client secrets out of Git; use CI secret injection where possible.

## GitHub Actions (Plan/Apply)

Workflow: `.github/workflows/terraform-infra.yml`

Required repository secrets:

- `AWS_ROLE_TO_ASSUME`

Optional backend secrets (recommended for shared state):

- `TF_STATE_BUCKET`
- `TF_STATE_LOCK_TABLE`
- `TF_STATE_KEY_PREFIX`

Workflow inputs:

- `action`: `plan` or `apply`
- `workspace`: Terraform workspace (for example `prod`)
- `tfvars_file`: file under `infra/` (for example `terraform.tfvars`)
