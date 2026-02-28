variable "project_name" {
  description = "Project prefix for resource names"
  type        = string
  default     = "r8r"
}

variable "aws_region" {
  description = "AWS region for deployment"
  type        = string
  default     = "us-east-1"
}

variable "vpc_cidr" {
  description = "CIDR block for VPC"
  type        = string
  default     = "10.42.0.0/16"
}

variable "public_subnet_cidrs" {
  description = "CIDRs for public subnets"
  type        = list(string)
  default     = ["10.42.0.0/24", "10.42.1.0/24"]

  validation {
    condition     = length(var.public_subnet_cidrs) == 2
    error_message = "public_subnet_cidrs must contain exactly 2 CIDR blocks."
  }
}

variable "private_subnet_cidrs" {
  description = "CIDRs for private subnets"
  type        = list(string)
  default     = ["10.42.10.0/24", "10.42.11.0/24"]

  validation {
    condition     = length(var.private_subnet_cidrs) == 2
    error_message = "private_subnet_cidrs must contain exactly 2 CIDR blocks."
  }
}

variable "clients" {
  description = "Per-client ECS service definitions"
  type = map(object({
    domain_name       = string
    image             = string
    cpu               = number
    memory            = number
    desired_count     = optional(number, 1)
    cors_origins      = optional(list(string), [])
    rust_log          = optional(string, "r8r=info")
    r8r_api_key       = optional(string)
    r8r_monitor_token = optional(string)
    autoscaling = optional(object({
      enabled            = optional(bool, false)
      min_capacity       = optional(number, 1)
      max_capacity       = optional(number, 2)
      cpu_target         = optional(number, 60)
      memory_target      = optional(number, 75)
      scale_in_cooldown  = optional(number, 120)
      scale_out_cooldown = optional(number, 60)
    }), {})
  }))

  validation {
    condition     = length(var.clients) > 0
    error_message = "clients must include at least one client entry."
  }
}

variable "enable_https" {
  description = "Enable HTTPS listener on ALB"
  type        = bool
  default     = true
}

variable "acm_certificate_arn" {
  description = "Existing ACM certificate ARN for ALB HTTPS. Leave empty to create/manage via Route53 DNS validation."
  type        = string
  default     = ""
}

variable "route53_zone_id" {
  description = "Route53 hosted zone ID for DNS validation and client domain alias records."
  type        = string
  default     = ""
}

variable "create_client_dns_records" {
  description = "Create Route53 A/AAAA alias records for each client domain."
  type        = bool
  default     = true
}

variable "alb_ingress_cidrs" {
  description = "CIDR blocks allowed to reach ALB (ports 80/443)"
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "alb_ssl_policy" {
  description = "TLS policy for ALB HTTPS listener"
  type        = string
  default     = "ELBSecurityPolicy-TLS13-1-2-2021-06"
}

variable "tags" {
  description = "Global tags applied to all resources"
  type        = map(string)
  default = {
    managed_by = "terraform"
    project    = "r8r"
  }
}
