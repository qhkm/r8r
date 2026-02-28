variable "project_name" {
  type = string
}

variable "client_id" {
  type = string
}

variable "domain_name" {
  type = string
}

variable "image" {
  type = string
}

variable "cpu" {
  type = number
}

variable "memory" {
  type = number
}

variable "desired_count" {
  type    = number
  default = 1
}

variable "cors_origins" {
  type    = list(string)
  default = []
}

variable "rust_log" {
  type    = string
  default = "r8r=info"
}

variable "api_key_secret_value" {
  type      = string
  default   = null
  sensitive = true
}

variable "monitor_token_secret_value" {
  type      = string
  default   = null
  sensitive = true
}

variable "cluster_id" {
  type = string
}

variable "cluster_name" {
  type = string
}

variable "listener_arn" {
  type = string
}

variable "listener_priority" {
  type = number
}

variable "aws_region" {
  type = string
}

variable "vpc_id" {
  type = string
}

variable "private_subnet_ids" {
  type = list(string)
}

variable "task_security_group_ids" {
  type = list(string)
}

variable "efs_file_system_id" {
  type = string
}

variable "execution_role_arn" {
  type = string
}

variable "task_role_arn" {
  type = string
}

variable "autoscaling" {
  type = object({
    enabled            = optional(bool, false)
    min_capacity       = optional(number, 1)
    max_capacity       = optional(number, 2)
    cpu_target         = optional(number, 60)
    memory_target      = optional(number, 75)
    scale_in_cooldown  = optional(number, 120)
    scale_out_cooldown = optional(number, 60)
  })
  default = {}
}

variable "tags" {
  type    = map(string)
  default = {}
}
