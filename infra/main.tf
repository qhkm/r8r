locals {
  client_ids_sorted = sort(keys(var.clients))
  client_priorities = {
    for idx, id in local.client_ids_sorted :
    id => 100 + idx
  }
}

module "client_service" {
  for_each = var.clients

  source = "./modules/client_service"

  project_name               = var.project_name
  client_id                  = each.key
  domain_name                = each.value.domain_name
  image                      = each.value.image
  cpu                        = each.value.cpu
  memory                     = each.value.memory
  desired_count              = try(each.value.desired_count, 1)
  cors_origins               = try(each.value.cors_origins, [])
  rust_log                   = try(each.value.rust_log, "r8r=info")
  api_key_secret_value       = try(each.value.r8r_api_key, null)
  monitor_token_secret_value = try(each.value.r8r_monitor_token, null)
  autoscaling                = try(each.value.autoscaling, {})
  listener_priority          = local.client_priorities[each.key]
  aws_region                 = var.aws_region

  cluster_id              = aws_ecs_cluster.main.id
  cluster_name            = aws_ecs_cluster.main.name
  listener_arn            = var.enable_https ? aws_lb_listener.https[0].arn : aws_lb_listener.http.arn
  vpc_id                  = aws_vpc.main.id
  private_subnet_ids      = values(aws_subnet.private)[*].id
  task_security_group_ids = [aws_security_group.task.id]
  efs_file_system_id      = aws_efs_file_system.shared.id
  execution_role_arn      = aws_iam_role.ecs_execution.arn
  task_role_arn           = aws_iam_role.ecs_task.arn

  tags = merge(var.tags, {
    client_id = each.key
  })
}
