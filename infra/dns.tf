resource "aws_route53_record" "client_alias_a" {
  for_each = var.create_client_dns_records && var.route53_zone_id != "" ? var.clients : {}

  zone_id = var.route53_zone_id
  name    = each.value.domain_name
  type    = "A"

  alias {
    name                   = aws_lb.main.dns_name
    zone_id                = aws_lb.main.zone_id
    evaluate_target_health = true
  }
}

resource "aws_route53_record" "client_alias_aaaa" {
  for_each = var.create_client_dns_records && var.route53_zone_id != "" ? var.clients : {}

  zone_id = var.route53_zone_id
  name    = each.value.domain_name
  type    = "AAAA"

  alias {
    name                   = aws_lb.main.dns_name
    zone_id                = aws_lb.main.zone_id
    evaluate_target_health = true
  }
}
