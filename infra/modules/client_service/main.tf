resource "random_password" "api_key" {
  count   = var.api_key_secret_value == null ? 1 : 0
  length  = 48
  special = false
}

resource "random_password" "monitor_token" {
  count   = var.monitor_token_secret_value == null ? 1 : 0
  length  = 64
  special = false
}

locals {
  api_key_value       = var.api_key_secret_value != null ? var.api_key_secret_value : random_password.api_key[0].result
  monitor_token_value = var.monitor_token_secret_value != null ? var.monitor_token_secret_value : random_password.monitor_token[0].result
  cors_origins_csv    = length(var.cors_origins) > 0 ? join(",", var.cors_origins) : ""
  service_name        = "${var.project_name}-${var.client_id}"
  target_group_name   = substr(replace("${var.project_name}-${var.client_id}-${substr(md5(var.client_id), 0, 6)}", "_", "-"), 0, 32)

  autoscaling_enabled            = try(var.autoscaling.enabled, false)
  autoscaling_min_capacity       = try(var.autoscaling.min_capacity, 1)
  autoscaling_max_capacity       = try(var.autoscaling.max_capacity, max(var.desired_count, 2))
  autoscaling_cpu_target         = try(var.autoscaling.cpu_target, 60)
  autoscaling_memory_target      = try(var.autoscaling.memory_target, 75)
  autoscaling_scale_in_cooldown  = try(var.autoscaling.scale_in_cooldown, 120)
  autoscaling_scale_out_cooldown = try(var.autoscaling.scale_out_cooldown, 60)
}

resource "aws_cloudwatch_log_group" "r8r" {
  name              = "/ecs/${local.service_name}"
  retention_in_days = 30

  tags = var.tags
}

resource "aws_secretsmanager_secret" "api_key" {
  name = "${var.project_name}/${var.client_id}/r8r_api_key"

  tags = var.tags
}

resource "aws_secretsmanager_secret_version" "api_key" {
  secret_id     = aws_secretsmanager_secret.api_key.id
  secret_string = local.api_key_value
}

resource "aws_secretsmanager_secret" "monitor_token" {
  name = "${var.project_name}/${var.client_id}/r8r_monitor_token"

  tags = var.tags
}

resource "aws_secretsmanager_secret_version" "monitor_token" {
  secret_id     = aws_secretsmanager_secret.monitor_token.id
  secret_string = local.monitor_token_value
}

resource "aws_efs_access_point" "client" {
  file_system_id = var.efs_file_system_id

  posix_user {
    gid = 1000
    uid = 1000
  }

  root_directory {
    path = "/clients/${var.client_id}"

    creation_info {
      owner_gid   = 1000
      owner_uid   = 1000
      permissions = "0755"
    }
  }

  tags = var.tags
}

resource "aws_lb_target_group" "client" {
  name        = local.target_group_name
  port        = 8080
  protocol    = "HTTP"
  target_type = "ip"
  vpc_id      = var.vpc_id

  health_check {
    path                = "/api/health"
    healthy_threshold   = 2
    unhealthy_threshold = 3
    timeout             = 5
    interval            = 30
    matcher             = "200"
  }

  tags = var.tags
}

resource "aws_lb_listener_rule" "client" {
  listener_arn = var.listener_arn
  priority     = var.listener_priority

  action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.client.arn
  }

  condition {
    host_header {
      values = [var.domain_name]
    }
  }

  tags = var.tags
}

resource "aws_ecs_task_definition" "r8r" {
  family                   = local.service_name
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = tostring(var.cpu)
  memory                   = tostring(var.memory)
  execution_role_arn       = var.execution_role_arn
  task_role_arn            = var.task_role_arn

  container_definitions = jsonencode([
    {
      name      = "r8r"
      image     = var.image
      essential = true
      portMappings = [
        {
          containerPort = 8080
          hostPort      = 8080
          protocol      = "tcp"
        }
      ]
      environment = [
        { name = "R8R_DATA_DIR", value = "/data" },
        { name = "R8R_DATABASE_PATH", value = "/data/r8r.db" },
        { name = "R8R_DB", value = "/data/r8r.db" },
        { name = "R8R_CORS_ORIGINS", value = local.cors_origins_csv },
        { name = "RUST_LOG", value = var.rust_log }
      ]
      secrets = [
        { name = "R8R_API_KEY", valueFrom = aws_secretsmanager_secret.api_key.arn },
        { name = "R8R_MONITOR_TOKEN", valueFrom = aws_secretsmanager_secret.monitor_token.arn }
      ]
      mountPoints = [
        {
          sourceVolume  = "r8r-data"
          containerPath = "/data"
          readOnly      = false
        }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          awslogs-group         = aws_cloudwatch_log_group.r8r.name
          awslogs-region        = var.aws_region
          awslogs-stream-prefix = "ecs"
        }
      }
    }
  ])

  volume {
    name = "r8r-data"

    efs_volume_configuration {
      file_system_id     = var.efs_file_system_id
      transit_encryption = "ENABLED"

      authorization_config {
        access_point_id = aws_efs_access_point.client.id
        iam             = "ENABLED"
      }
    }
  }

  tags = var.tags
}

resource "aws_ecs_service" "r8r" {
  name            = local.service_name
  cluster         = var.cluster_id
  task_definition = aws_ecs_task_definition.r8r.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  deployment_minimum_healthy_percent = 50
  deployment_maximum_percent         = 200

  network_configuration {
    subnets          = var.private_subnet_ids
    security_groups  = var.task_security_group_ids
    assign_public_ip = false
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.client.arn
    container_name   = "r8r"
    container_port   = 8080
  }

  depends_on = [aws_lb_listener_rule.client]

  lifecycle {
    ignore_changes = [desired_count]
  }

  tags = var.tags
}

resource "aws_appautoscaling_target" "ecs_service" {
  count = local.autoscaling_enabled ? 1 : 0

  service_namespace  = "ecs"
  resource_id        = "service/${var.cluster_name}/${aws_ecs_service.r8r.name}"
  scalable_dimension = "ecs:service:DesiredCount"
  min_capacity       = local.autoscaling_min_capacity
  max_capacity       = local.autoscaling_max_capacity

  lifecycle {
    precondition {
      condition     = local.autoscaling_min_capacity <= local.autoscaling_max_capacity
      error_message = "autoscaling.min_capacity must be <= autoscaling.max_capacity."
    }
  }
}

resource "aws_appautoscaling_policy" "cpu_target" {
  count = local.autoscaling_enabled ? 1 : 0

  name               = "${local.service_name}-cpu-target"
  service_namespace  = aws_appautoscaling_target.ecs_service[0].service_namespace
  resource_id        = aws_appautoscaling_target.ecs_service[0].resource_id
  scalable_dimension = aws_appautoscaling_target.ecs_service[0].scalable_dimension
  policy_type        = "TargetTrackingScaling"

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageCPUUtilization"
    }
    target_value       = local.autoscaling_cpu_target
    scale_in_cooldown  = local.autoscaling_scale_in_cooldown
    scale_out_cooldown = local.autoscaling_scale_out_cooldown
  }
}

resource "aws_appautoscaling_policy" "memory_target" {
  count = local.autoscaling_enabled ? 1 : 0

  name               = "${local.service_name}-memory-target"
  service_namespace  = aws_appautoscaling_target.ecs_service[0].service_namespace
  resource_id        = aws_appautoscaling_target.ecs_service[0].resource_id
  scalable_dimension = aws_appautoscaling_target.ecs_service[0].scalable_dimension
  policy_type        = "TargetTrackingScaling"

  target_tracking_scaling_policy_configuration {
    predefined_metric_specification {
      predefined_metric_type = "ECSServiceAverageMemoryUtilization"
    }
    target_value       = local.autoscaling_memory_target
    scale_in_cooldown  = local.autoscaling_scale_in_cooldown
    scale_out_cooldown = local.autoscaling_scale_out_cooldown
  }
}
