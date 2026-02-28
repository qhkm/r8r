output "alb_dns_name" {
  description = "Public ALB hostname"
  value       = aws_lb.main.dns_name
}

output "alb_http_url" {
  description = "HTTP URL for ALB"
  value       = "http://${aws_lb.main.dns_name}"
}

output "alb_https_url" {
  description = "HTTPS URL for ALB (null if HTTPS disabled)"
  value       = var.enable_https ? "https://${aws_lb.main.dns_name}" : null
}

output "client_services" {
  description = "Per-client ECS service metadata"
  value = {
    for k, v in module.client_service :
    k => {
      ecs_service_name = v.ecs_service_name
      api_key_secret   = v.api_key_secret_arn
      monitor_secret   = v.monitor_token_secret_arn
    }
  }
}
