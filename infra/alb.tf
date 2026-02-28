locals {
  client_domains = sort(distinct([for c in values(var.clients) : c.domain_name]))

  managed_cert_enabled = var.enable_https && var.acm_certificate_arn == "" && var.route53_zone_id != ""

  alb_certificate_arn = var.acm_certificate_arn != "" ? var.acm_certificate_arn : try(aws_acm_certificate_validation.managed[0].certificate_arn, "")
}

resource "aws_security_group" "alb" {
  name        = "${var.project_name}-alb-sg"
  description = "Public ingress to r8r ALB"
  vpc_id      = aws_vpc.main.id

  ingress {
    description = "HTTP"
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = var.alb_ingress_cidrs
  }

  ingress {
    description = "HTTPS"
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = var.alb_ingress_cidrs
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = var.tags
}

resource "aws_lb" "main" {
  name               = "${var.project_name}-alb"
  internal           = false
  load_balancer_type = "application"
  security_groups    = [aws_security_group.alb.id]
  subnets            = values(aws_subnet.public)[*].id

  tags = var.tags
}

resource "aws_acm_certificate" "managed" {
  count = local.managed_cert_enabled ? 1 : 0

  domain_name               = local.client_domains[0]
  subject_alternative_names = slice(local.client_domains, 1, length(local.client_domains))
  validation_method         = "DNS"

  lifecycle {
    create_before_destroy = true
  }

  tags = var.tags
}

resource "aws_route53_record" "acm_validation" {
  for_each = local.managed_cert_enabled ? {
    for dvo in aws_acm_certificate.managed[0].domain_validation_options :
    dvo.domain_name => {
      name   = dvo.resource_record_name
      type   = dvo.resource_record_type
      record = dvo.resource_record_value
    }
  } : {}

  zone_id = var.route53_zone_id
  name    = each.value.name
  type    = each.value.type
  ttl     = 60
  records = [each.value.record]
}

resource "aws_acm_certificate_validation" "managed" {
  count = local.managed_cert_enabled ? 1 : 0

  certificate_arn         = aws_acm_certificate.managed[0].arn
  validation_record_fqdns = [for r in aws_route53_record.acm_validation : r.fqdn]
}

resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.main.arn
  port              = 80
  protocol          = "HTTP"

  default_action {
    type = var.enable_https ? "redirect" : "fixed-response"

    dynamic "redirect" {
      for_each = var.enable_https ? [1] : []
      content {
        port        = "443"
        protocol    = "HTTPS"
        status_code = "HTTP_301"
      }
    }

    dynamic "fixed_response" {
      for_each = var.enable_https ? [] : [1]
      content {
        content_type = "text/plain"
        message_body = "r8r: host rule not configured"
        status_code  = "404"
      }
    }
  }
}

resource "aws_lb_listener" "https" {
  count = var.enable_https ? 1 : 0

  load_balancer_arn = aws_lb.main.arn
  port              = 443
  protocol          = "HTTPS"
  ssl_policy        = var.alb_ssl_policy
  certificate_arn   = local.alb_certificate_arn

  default_action {
    type = "fixed-response"

    fixed_response {
      content_type = "text/plain"
      message_body = "r8r: host rule not configured"
      status_code  = "404"
    }
  }

  lifecycle {
    precondition {
      condition     = local.alb_certificate_arn != ""
      error_message = "enable_https=true requires acm_certificate_arn or route53_zone_id for managed ACM."
    }
  }
}
