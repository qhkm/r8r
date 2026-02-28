output "ecs_service_name" {
  value = aws_ecs_service.r8r.name
}

output "api_key_secret_arn" {
  value = aws_secretsmanager_secret.api_key.arn
}

output "monitor_token_secret_arn" {
  value = aws_secretsmanager_secret.monitor_token.arn
}
